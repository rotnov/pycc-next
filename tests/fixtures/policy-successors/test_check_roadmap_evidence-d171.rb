#!/usr/bin/env ruby
# frozen_string_literal: true

require "fileutils"
require "minitest/autorun"
require "open3"
require "pathname"
require "psych"
require "rbconfig"
require "tmpdir"

CHECKER_UNDER_TEST =
  if Pathname(__dir__).basename.to_s == "scripts"
    Pathname(__dir__) / "check_roadmap_evidence.rb"
  else
    Pathname(__dir__) / "check_roadmap_evidence-d171.rb"
  end
require CHECKER_UNDER_TEST.to_s

class RoadmapEvidenceCliTest < Minitest::Test
  REPOSITORY_ROOT =
    if Pathname(__dir__).basename.to_s == "scripts"
      Pathname(__dir__).parent
    else
      Pathname(__dir__).parent.parent.parent
    end
  CHECKER = CHECKER_UNDER_TEST
  ACTIVE_D100_COMPOSE_D91_D99_WORKFLOW =
    REPOSITORY_ROOT / ".github/workflows/ci.yml"
  D171_WORKFLOW_FIXTURE =
    REPOSITORY_ROOT / "tests/fixtures/policy-successors/ci-d171.yml"
  RETIRED_D51_PAIRED_WORKFLOW =
    REPOSITORY_ROOT / "tests/fixtures/d51-paired-ci.yml"
  D56_SOURCE_AWARE_WORKFLOW_FIXTURE =
    REPOSITORY_ROOT / "tests/fixtures/d56-source-aware-ci.yml"
  D62_REPLICATED_PAIRED_WORKFLOW_FIXTURE =
    REPOSITORY_ROOT / "tests/fixtures/d62-replicated-paired-ci.yml"
  D80_CONFORMANCE_ORACLE_WORKFLOW_FIXTURE =
    REPOSITORY_ROOT / "tests/fixtures/d80-conformance-oracle-ci.yml"
  D84_THROUGHPUT_FLOOR_WORKFLOW_FIXTURE =
    REPOSITORY_ROOT / "tests/fixtures/d84-throughput-floor-ci.yml"
  D91_RELAX_FRONTEND_PERF_MANIFEST_WORKFLOW_FIXTURE =
    REPOSITORY_ROOT /
    "tests/fixtures/d91-relax-frontend-perf-manifest-ci.yml"
  D99_VCPKG_LIBXML2_CACHE_WORKFLOW_FIXTURE =
    REPOSITORY_ROOT /
    "tests/fixtures/d99-vcpkg-libxml2-cache-ci.yml"
  D100_COMPOSED_WORKFLOW_FIXTURE =
    REPOSITORY_ROOT /
    "tests/fixtures/d100-compose-d91-d99-ci.yml"
  D112_UBUNTU_FRONTEND_PERF_WORKFLOW_FIXTURE =
    REPOSITORY_ROOT / "tests/fixtures/d112-ubuntu-frontend-perf-ci.yml"
  D114_FRONTEND_PERF_THRESHOLD_WORKFLOW_FIXTURE =
    REPOSITORY_ROOT /
    "tests/fixtures/d114-frontend-perf-threshold-ci.yml"
  D229_PAGES_PERFORMANCE_WORKFLOW_FIXTURE =
    REPOSITORY_ROOT /
    "tests/fixtures/policy-successors/ci.yml"
  D199_PAGES_ACCESSIBILITY_WORKFLOW_FIXTURE =
    REPOSITORY_ROOT /
    "tests/fixtures/policy-successors/ci-d199.yml"
  D211_COVERAGE_BADGE_BINDING_WORKFLOW_FIXTURE =
    REPOSITORY_ROOT /
    "tests/fixtures/policy-successors/ci-d211.yml"
  PY3147_ORACLE_WORKFLOW_FIXTURE =
    REPOSITORY_ROOT /
    "tests/fixtures/policy-successors/ci-python-3147.yml"
  COVERAGE_STEP_HEADER =
    "      - name: Hard coverage gate — 100% lines + regions (D-014)"
  COVERAGE_COMMAND =
    "run_isolated \"$TRUSTED_COV\" llvm-cov --workspace " \
    "--fail-under-lines 100 --fail-under-regions 100"

  D171_COMPILER_JOBS = %w[
    build-test-coverage
    native-build-test
    cross-compile-build
    cross-compile-verify
    frontend-perf-measure
    frontend-perf-gate
  ].freeze
  D171_PAGES_JOBS = %w[pages-performance pages-accessibility].freeze

  def d171_workflow
    stream = Psych.parse_stream(
      D171_WORKFLOW_FIXTURE.read,
      filename: D171_WORKFLOW_FIXTURE.to_s
    )
    workflow = yaml_value(stream.children.first.root, D171_WORKFLOW_FIXTURE.to_s)
    yield workflow if block_given?
    workflow
  end

  def assert_d171_routing_rejected(workflow, label, expected_context:)
    error = assert_raises(RoadmapEvidenceError, label) do
      validate_d171_ci_routing(workflow.to_yaml, "ci-d171.yml")
    end
    assert_includes error.message, expected_context, label
  end

  def d171_checkout_locations(workflow)
    workflow.fetch("jobs").flat_map do |job_name, job|
      job.fetch("steps", []).each_with_index.each_with_object([]) do |(step, index), locations|
        if step.fetch("uses", "").start_with?("actions/checkout@")
          locations << [job_name, index]
        end
      end
    end
  end

  def coverage_workflow(command = COVERAGE_COMMAND)
    <<~YAML
      on:
        pull_request:
      env:
        CARGO_LLVM_COV_VERSION: "0.8.7"
        LLVM_VERSION: "22.1.1"
      jobs:
        build-test-coverage:
          runs-on: macos-14
          steps:
            - uses: actions/checkout@d23441a48e516b6c34aea4fa41551a30e30af803
              with:
                persist-credentials: false
            - name: Show pinned toolchain
              run: rustup show
            - name: Install LLVM 22 (D-015)
              run: brew install llvm@22
            - name: Install llvm-tools-preview
              run: rustup component add llvm-tools-preview
            - name: Add x86_64-apple-darwin Rust target
              run: rustup target add x86_64-apple-darwin
            - name: Hard coverage gate — 100% lines + regions (D-014)
              run: |
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
                #{command}
                rm "$GITHUB_WORKSPACE/target"
                printf 'LLVM_SYS_221_PREFIX=%s\\n' "$LLVM_SYS_221_PREFIX_VALUE" >> "$GITHUB_ENV"
    YAML
  end

  def run_checker(roadmap:, workflow:)
    Dir.mktmpdir do |directory|
      root = Pathname(directory)
      FileUtils.mkdir_p(root / "docs")
      FileUtils.mkdir_p(root / ".github/workflows")
      (root / "docs/ROADMAP.md").write(roadmap)
      (root / ".github/workflows/ci.yml").write(workflow)
      return Open3.capture3(RbConfig.ruby, CHECKER.to_s, root.to_s)
    end
  end

  def paired_perf_workflow
    jobs = {
      "frontend-perf-measure" =>
        Marshal.load(Marshal.dump(PAIRED_PERF_MEASURE_JOB)),
      "frontend-perf-gate" =>
        Marshal.load(Marshal.dump(PAIRED_PERF_GATE_JOB)),
      "ci-gate" =>
        Marshal.load(Marshal.dump(PAIRED_PERF_CI_GATE_JOB))
    }
    yield jobs if block_given?
    { "jobs" => jobs }.to_yaml
  end

  def source_aware_perf_workflow
    jobs = {
      "frontend-perf-measure" =>
        Marshal.load(Marshal.dump(D56_SOURCE_AWARE_PERF_MEASURE_JOB)),
      "frontend-perf-gate" =>
        Marshal.load(Marshal.dump(D56_SOURCE_AWARE_PERF_GATE_JOB)),
      "ci-gate" =>
        Marshal.load(Marshal.dump(PAIRED_PERF_CI_GATE_JOB))
    }
    yield jobs if block_given?
    { "jobs" => jobs }.to_yaml
  end

  def replicated_perf_workflow
    jobs = {
      "frontend-perf-measure" =>
        Marshal.load(Marshal.dump(REPLICATED_PERF_MEASURE_JOB)),
      "frontend-perf-gate" =>
        Marshal.load(Marshal.dump(REPLICATED_PERF_GATE_JOB)),
      "ci-gate" =>
        Marshal.load(Marshal.dump(PAIRED_PERF_CI_GATE_JOB))
    }
    yield jobs if block_given?
    { "jobs" => jobs }.to_yaml
  end

  def d112_ubuntu_frontend_perf_workflow
    jobs = {
      "frontend-perf-measure" =>
        Marshal.load(Marshal.dump(D112_UBUNTU_FRONTEND_PERF_MEASURE_JOB)),
      "frontend-perf-gate" =>
        Marshal.load(Marshal.dump(D112_UBUNTU_FRONTEND_PERF_GATE_JOB)),
      "ci-gate" =>
        Marshal.load(Marshal.dump(PAIRED_PERF_CI_GATE_JOB))
    }
    yield jobs if block_given?
    { "jobs" => jobs }.to_yaml
  end

  def d114_raised_threshold_frontend_perf_workflow
    jobs = {
      "frontend-perf-measure" =>
        Marshal.load(Marshal.dump(D112_UBUNTU_FRONTEND_PERF_MEASURE_JOB)),
      "frontend-perf-gate" =>
        Marshal.load(Marshal.dump(D114_RAISED_THRESHOLD_FRONTEND_PERF_GATE_JOB)),
      "ci-gate" =>
        Marshal.load(Marshal.dump(PAIRED_PERF_CI_GATE_JOB))
    }
    yield jobs if block_given?
    { "jobs" => jobs }.to_yaml
  end

  def without_workflow_jobs(workflow, *job_names)
    skipping = false
    workflow.lines.reject do |line|
      if (match = /^  (?<name>[a-z0-9-]+):\s*$/.match(line))
        skipping = job_names.include?(match[:name])
      end
      skipping
    end.join
  end

  def roadmap_with_tier1_claim(state)
    claim =
      "The five-target native CI matrix and one cross-host compilation " \
      "path are live on `main`."
    item =
      case state
      when :unchecked
        "- [ ] #{claim}"
      when :absent
        nil
      else
        raise ArgumentError, "unsupported Tier-1 claim state: #{state}"
      end
    ["# pycc Roadmap", item].compact.join("\n") + "\n"
  end

  def retired_d48_workflow
    coverage_workflow.sub(
      "jobs:\n",
      <<~YAML
        jobs:
          frontend-perf-measure:
            runs-on: macos-14
            steps:
              - uses: actions/checkout@d23441a48e516b6c34aea4fa41551a30e30af803
              - name: Run frontend benchmark
                run: cargo bench --bench check_bench -- --save-baseline current
          frontend-perf-gate:
            needs: frontend-perf-measure
            runs-on: macos-14
            steps:
              - name: Compare against canonical main baseline
                run: ruby scripts/check_perf_regression.rb current.json previous.json
      YAML
    )
  end

  def run_paired_predecessor_resolution(
    event_name:,
    pr_base_sha: "exact-base-sha",
    push_before_sha: "exact-before-sha"
  )
    Dir.mktmpdir do |directory|
      output = Pathname(directory) / "github-output"
      output.write("")
      env = {
        "GITHUB_OUTPUT" => output.to_s,
        "GITHUB_EVENT_NAME" => event_name,
        "PR_BASE_SHA" => pr_base_sha,
        "PUSH_BEFORE_SHA" => push_before_sha
      }
      stdout, stderr, status = Open3.capture3(
        env,
        "bash",
        "-s",
        stdin_data: PAIRED_PERF_PREDECESSOR_SCRIPT,
        chdir: directory
      )
      return [stdout, stderr, status, output.read]
    end
  end

  def run_paired_timing_requirement
    Dir.mktmpdir do |directory|
      root =
        Pathname(directory) /
        "target/criterion/pycc_check_frontend_fixture"
      FileUtils.mkdir_p(root)
      yield root
      return Open3.capture3(
        "bash",
        "-s",
        stdin_data: PAIRED_PERF_REQUIRE_SCRIPT,
        chdir: directory
      )
    end
  end

  def run_replicated_timing_requirement
    Dir.mktmpdir do |directory|
      root =
        Pathname(directory) /
        "target/criterion/pycc_check_frontend_fixture"
      FileUtils.mkdir_p(root)
      yield root
      return Open3.capture3(
        "bash",
        "-s",
        stdin_data: REPLICATED_PERF_REQUIRE_SCRIPT,
        chdir: directory
      )
    end
  end

  def run_paired_artifact_identity_requirement(
    previous_artifact_id:,
    current_artifact_id:
  )
    Open3.capture3(
      {
        "PREVIOUS_ARTIFACT_ID" => previous_artifact_id,
        "CURRENT_ARTIFACT_ID" => current_artifact_id
      },
      "bash",
      "-s",
      stdin_data: PAIRED_PERF_ARTIFACT_ID_REQUIRE_SCRIPT
    )
  end


  def run_executable_input_classifier
    Dir.mktmpdir do |directory|
      root = Pathname(directory)
      %w[previous current].each do |revision|
        FileUtils.mkdir_p(root / revision / "src")
        FileUtils.mkdir_p(root / revision / "crates")
      end
      yield root
      output = root / "github-output"
      output.write("")
      stdout, stderr, status = Open3.capture3(
        { "GITHUB_OUTPUT" => output.to_s },
        "bash",
        "-s",
        stdin_data: D56_EXECUTABLE_INPUT_IDENTITY_SCRIPT,
        chdir: root.to_s
      )
      return [stdout, stderr, status, output.read]
    end
  end

  # D-091: same shape as run_executable_input_classifier, but exercises the
  # actual D91_EXECUTABLE_INPUT_IDENTITY_SCRIPT (which adds Cargo.toml,
  # Cargo.lock, and build.rs to the classified path list) instead of D56's
  # src/crates-only predecessor.
  def run_d91_executable_input_classifier
    Dir.mktmpdir do |directory|
      root = Pathname(directory)
      %w[previous current].each do |revision|
        FileUtils.mkdir_p(root / revision / "src")
        FileUtils.mkdir_p(root / revision / "crates")
      end
      yield root
      output = root / "github-output"
      output.write("")
      stdout, stderr, status = Open3.capture3(
        { "GITHUB_OUTPUT" => output.to_s },
        "bash",
        "-s",
        stdin_data: D91_EXECUTABLE_INPUT_IDENTITY_SCRIPT,
        chdir: root.to_s
      )
      return [stdout, stderr, status, output.read]
    end
  end

  # D-091: exercises the real D91_VERIFY_REVISIONS_SCRIPT end to end,
  # including its `git -C previous/current rev-parse HEAD` preamble, against
  # real (throwaway) git repositories -- not just a substring check of the
  # constant's text -- so a change that silently breaks the bench-manifest
  # fingerprint's awk/grep logic actually fails a test, per the same
  # measurement-integrity finding this fingerprint was added to fix.
  def run_d91_verify_revisions
    Dir.mktmpdir do |directory|
      root = Pathname(directory)
      %w[previous current].each do |revision|
        FileUtils.mkdir_p(root / revision)
      end
      yield root
      shas = {}
      %w[previous current].each do |revision|
        repo = (root / revision).to_s
        Open3.capture2("git", "-C", repo, "init", "-q")
        Open3.capture2("git", "-C", repo, "config", "user.email", "test@example.invalid")
        Open3.capture2("git", "-C", repo, "config", "user.name", "Test")
        Open3.capture2("git", "-C", repo, "add", "-A")
        _out, commit_err, commit_status =
          Open3.capture3("git", "-C", repo, "commit", "-q", "-m", "content")
        raise commit_err unless commit_status.success?

        sha, = Open3.capture2("git", "-C", repo, "rev-parse", "HEAD")
        shas[revision] = sha.strip
      end
      env = {
        "EXPECTED_PREDECESSOR_SHA" => shas.fetch("previous"),
        "EXPECTED_CURRENT_SHA" => shas.fetch("current")
      }
      return Open3.capture3(
        env,
        "bash",
        "-s",
        stdin_data: D91_VERIFY_REVISIONS_SCRIPT,
        chdir: root.to_s
      )
    end
  end

  D91_BENCH_MANIFEST_TAIL = <<~TOML
    [dev-dependencies]
    serde_json = "1"
    criterion = { version = "0.8.2", features = ["html_reports"] }

    [[bench]]
    name = "check_bench"
    harness = false
  TOML

  def d91_cargo_toml(dependencies_extra: "", bench_manifest_tail: D91_BENCH_MANIFEST_TAIL)
    <<~TOML
      [package]
      name = "pycc"
      version = "0.1.0"

      [dependencies]
      clap = { version = "4", features = ["derive"] }
      #{dependencies_extra}
      #{bench_manifest_tail}
    TOML
  end

  def run_executable_input_identity_requirement(value)
    Open3.capture3(
      { "EXECUTABLE_INPUTS_EQUAL" => value },
      "bash",
      "-s",
      stdin_data: D56_EXECUTABLE_INPUT_IDENTITY_REQUIRE_SCRIPT
    )
  end

  def test_rejects_false_completed_item_without_evidence
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      ## Current delivery status

      ### v0.1 acceptance checklist

      - [x] `pycc check` processes 1k LOC in under 50 ms.
    MARKDOWN

    _stdout, stderr, status = run_checker(roadmap: roadmap, workflow: "jobs: {}\n")

    refute status.success?
    assert_includes stderr, "checked roadmap item is missing an evidence marker"
  end

  def test_rejects_all_checked_markdown_list_bullets_without_evidence
    ["*", "+", "1."].each do |bullet|
      roadmap = <<~MARKDOWN
        # pycc Roadmap

        ## Current delivery status

        ### v0.1 acceptance checklist

        #{bullet} [x] `pycc check` processes 1k LOC in under 50 ms.
      MARKDOWN

      _stdout, stderr, status = run_checker(roadmap: roadmap, workflow: "jobs: {}\n")

      refute status.success?, "expected #{bullet.inspect} task item to be checked"
      assert_includes stderr, "checked roadmap item is missing an evidence marker"
    end
  end

  def test_rejects_checked_items_nested_in_markdown_blockquotes_without_evidence
    ["> -", "> > 1)"].each do |prefix|
      roadmap = <<~MARKDOWN
        # pycc Roadmap

        ## Current delivery status

        ### v0.1 acceptance checklist

        #{prefix} [x] `pycc check` processes 1k LOC in under 50 ms.
      MARKDOWN

      _stdout, stderr, status = run_checker(roadmap: roadmap, workflow: "jobs: {}\n")

      refute status.success?, "expected #{prefix.inspect} task item to be checked"
      assert_includes stderr, "checked roadmap item is missing an evidence marker"
    end
  end

  def test_rejects_checked_items_nested_under_list_containers_without_evidence
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      ## Current delivery status

      ### v0.1 acceptance checklist

      - Grouped acceptance claims:

          - [x] `pycc check` processes 1k LOC in under 50 ms.
    MARKDOWN

    _stdout, stderr, status = run_checker(roadmap: roadmap, workflow: "jobs: {}\n")

    refute status.success?
    assert_includes stderr, "checked roadmap item is missing an evidence marker"
  end

  def test_rejects_evidence_marker_attached_to_the_wrong_claim
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      ## Current delivery status

      ### v0.1 acceptance checklist

      - [x] `pycc check` processes 1k LOC in under 50 ms. <!-- roadmap-evidence: ci-build-test-coverage-100 -->
    MARKDOWN

    _stdout, stderr, status = run_checker(
      roadmap: roadmap,
      workflow: coverage_workflow
    )

    refute status.success?
    assert_includes stderr, "does not prove this roadmap claim"
  end

  def test_rejects_coverage_evidence_moved_outside_the_v0_1_checklist
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      ## v1.0 — spec freeze

      ### v0.1 acceptance checklist

      - [x] The 100% line and region coverage gate is required and green for the current slice. <!-- roadmap-evidence: ci-build-test-coverage-100 -->
    MARKDOWN

    _stdout, stderr, status = run_checker(
      roadmap: roadmap,
      workflow: coverage_workflow
    )

    refute status.success?
    assert_includes stderr, "must appear under the expected roadmap section"
  end

  def test_rejects_coverage_evidence_after_a_blockquoted_heading_changes_the_section
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      ## Current delivery status

      ### v0.1 acceptance checklist

      > #### Other milestone
      >
      > - [x] The 100% line and region coverage gate is required and green for the current slice. <!-- roadmap-evidence: ci-build-test-coverage-100 -->
    MARKDOWN

    _stdout, stderr, status = run_checker(
      roadmap: roadmap,
      workflow: coverage_workflow
    )

    refute status.success?
    assert_includes stderr, "must appear under the expected roadmap section"
  end

  def test_rejects_coverage_evidence_after_a_list_item_heading_changes_the_section
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      ## Current delivery status

      ### v0.1 acceptance checklist

      - #### Other milestone
        - [x] The 100% line and region coverage gate is required and green for the current slice. <!-- roadmap-evidence: ci-build-test-coverage-100 -->
    MARKDOWN

    _stdout, stderr, status = run_checker(
      roadmap: roadmap,
      workflow: coverage_workflow
    )

    refute status.success?
    assert_includes stderr, "must appear under the expected roadmap section"
  end

  def test_rejects_checked_item_continuing_an_empty_list_marker_without_evidence
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      ## Current delivery status

      ### v0.1 acceptance checklist

      -
        [x] `pycc check` processes 1k LOC in under 50 ms.
    MARKDOWN

    _stdout, stderr, status = run_checker(roadmap: roadmap, workflow: "jobs: {}\n")

    refute status.success?
    assert_includes stderr, "checked roadmap item is missing an evidence marker"
  end

  def test_ignores_headings_hidden_in_fences_and_html_comments
    hidden_sections = [
      <<~MARKDOWN,
        ```markdown
        # pycc Roadmap

        ## Current delivery status

        ### v0.1 acceptance checklist
        ```
      MARKDOWN
      <<~MARKDOWN
        <!--
        # pycc Roadmap

        ## Current delivery status

        ### v0.1 acceptance checklist
        -->
      MARKDOWN
    ]

    hidden_sections.each do |hidden_section|
      roadmap = <<~MARKDOWN
        # pycc Roadmap

        ## v1.0 — spec freeze

        #{hidden_section}
        - [x] The 100% line and region coverage gate is required and green for the current slice. <!-- roadmap-evidence: ci-build-test-coverage-100 -->
      MARKDOWN

      _stdout, stderr, status = run_checker(
        roadmap: roadmap,
        workflow: coverage_workflow
      )

      refute status.success?
      assert_includes stderr, "must appear under the expected roadmap section"
    end
  end

  def test_rejects_headings_hidden_inside_raw_html_blocks
    roadmap = <<~MARKDOWN
      # Wrong Roadmap

      <script>
      # pycc Roadmap

      ## Current delivery status

      ### v0.1 acceptance checklist
      </script>

      - [x] The 100% line and region coverage gate is required and green for the current slice. <!-- roadmap-evidence: ci-build-test-coverage-100 -->
    MARKDOWN

    _stdout, stderr, status = run_checker(
      roadmap: roadmap,
      workflow: coverage_workflow
    )

    refute status.success?
    assert_includes stderr, "raw HTML blocks are not supported"
  end

  def test_rejects_tab_indented_pseudo_headings
    roadmap = <<~MARKDOWN
      \t# pycc Roadmap

      \t## Current delivery status

      \t### v0.1 acceptance checklist

      - [x] The 100% line and region coverage gate is required and green for the current slice. <!-- roadmap-evidence: ci-build-test-coverage-100 -->
    MARKDOWN

    _stdout, stderr, status = run_checker(
      roadmap: roadmap,
      workflow: coverage_workflow
    )

    refute status.success?
    assert_includes stderr, "must appear under the expected roadmap section"
  end

  def test_ignores_checked_items_hidden_in_fences_and_html_comments
    hidden_items = [
      "```\n- [x] Hidden example without evidence.\n```\n",
      "<!--\n- [x] Hidden note without evidence.\n-->\n"
    ]

    hidden_items.each do |hidden_item|
      stdout, stderr, status = run_checker(
        roadmap: "# pycc Roadmap\n\n#{hidden_item}",
        workflow: ACTIVE_D100_COMPOSE_D91_D99_WORKFLOW.read
      )

      assert status.success?, stderr
      assert_includes stdout, "Roadmap evidence policy passed."
    end
  end

  def test_ignores_checked_items_inside_blockquoted_fences
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      > ```markdown
      > - [x] Quoted code example without evidence.
      > ```
    MARKDOWN

    stdout, stderr, status = run_checker(
      roadmap: roadmap,
      workflow: ACTIVE_D100_COMPOSE_D91_D99_WORKFLOW.read
    )

    assert status.success?, stderr
    assert_includes stdout, "Roadmap evidence policy passed."
  end

  def test_ignores_checked_items_inside_fences_nested_under_list_items
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      - Example:

          ```markdown
          - [x] Nested code example without evidence.
          ```
    MARKDOWN

    stdout, stderr, status = run_checker(
      roadmap: roadmap,
      workflow: ACTIVE_D100_COMPOSE_D91_D99_WORKFLOW.read
    )

    assert status.success?, stderr
    assert_includes stdout, "Roadmap evidence policy passed."
  end

  def test_ignores_checked_items_rendered_as_indented_code
    ["    - [x] Root code example.\n", ">     - [x] Quoted code example.\n"].each do |example|
      stdout, stderr, status = run_checker(
        roadmap: "# pycc Roadmap\n\n#{example}",
        workflow: ACTIVE_D100_COMPOSE_D91_D99_WORKFLOW.read
      )

      assert status.success?, stderr
      assert_includes stdout, "Roadmap evidence policy passed."
    end
  end

  def test_rejects_setext_headings_that_change_the_rendered_section
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      ## Current delivery status

      ### v0.1 acceptance checklist

      Wrong rendered root
      ===================

      - [x] The 100% line and region coverage gate is required and green for the current slice. <!-- roadmap-evidence: ci-build-test-coverage-100 -->
    MARKDOWN

    _stdout, stderr, status = run_checker(
      roadmap: roadmap,
      workflow: coverage_workflow
    )

    refute status.success?
    assert_includes stderr, "Setext headings are not supported"
  end

  def test_rejects_an_unknown_evidence_marker
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      ## Current delivery status

      ### v0.1 acceptance checklist

      - [x] The 100% line and region coverage gate is required and green for the current slice. <!-- roadmap-evidence: invented-proof -->
    MARKDOWN

    _stdout, stderr, status = run_checker(
      roadmap: roadmap,
      workflow: coverage_workflow
    )

    refute status.success?
    assert_includes stderr, 'unknown roadmap evidence "invented-proof"'
  end

  def test_rejects_multiple_evidence_markers_on_one_checked_item
    repository_root = REPOSITORY_ROOT
    workflow = (repository_root / ".github/workflows/ci.yml").read
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      ## Current delivery status

      ### v0.1 acceptance checklist

      - [x] The five-target native CI matrix and one cross-host compilation path are live on `main`. <!-- roadmap-evidence: ci-tier1-cross-compile --> <!-- roadmap-evidence: invented-proof -->
    MARKDOWN

    _stdout, stderr, status = run_checker(roadmap: roadmap, workflow: workflow)

    refute status.success?
    assert_includes stderr, "must contain exactly one evidence marker"
  end

  def test_accepts_reviewed_tier1_matrix_evidence
    repository_root = REPOSITORY_ROOT
    workflow = (repository_root / ".github/workflows/ci.yml").read
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      ## Current delivery status

      ### v0.1 acceptance checklist

      - [x] The five-target native CI matrix and one cross-host compilation path are live on `main`. <!-- roadmap-evidence: ci-tier1-cross-compile -->
    MARKDOWN

    stdout, stderr, status = run_checker(roadmap: roadmap, workflow: workflow)

    assert status.success?, stderr
    assert_includes stdout, "Roadmap evidence policy passed."
  end

  def test_accepts_conformance_tier1_evidence
    repository_root = REPOSITORY_ROOT
    workflow = (repository_root / ".github/workflows/ci.yml").read
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      ## Current delivery status

      ### v0.1 acceptance checklist

      - [x] `fib` and `mandelbrot-ascii` compile and match CPython output on all five Tier-1 targets. <!-- roadmap-evidence: conformance-fib-mandelbrot-tier1 -->
    MARKDOWN

    stdout, stderr, status = run_checker(roadmap: roadmap, workflow: workflow)

    assert status.success?, stderr
    assert_includes stdout, "Roadmap evidence policy passed."
  end

  def test_rejects_conformance_tier1_evidence_with_the_wrong_claim
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      ## Current delivery status

      ### v0.1 acceptance checklist

      - [x] `fib` and `mandelbrot-ascii` compile on all five Tier-1 targets. <!-- roadmap-evidence: conformance-fib-mandelbrot-tier1 -->
    MARKDOWN

    _stdout, stderr, status = run_checker(roadmap: roadmap, workflow: coverage_workflow)

    refute status.success?
    assert_includes stderr, "does not prove this roadmap claim"
  end

  def test_rejects_conformance_tier1_evidence_outside_the_v0_1_checklist
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      ## v1.0 — spec freeze

      ### v0.1 acceptance checklist

      - [x] `fib` and `mandelbrot-ascii` compile and match CPython output on all five Tier-1 targets. <!-- roadmap-evidence: conformance-fib-mandelbrot-tier1 -->
    MARKDOWN

    _stdout, stderr, status = run_checker(roadmap: roadmap, workflow: coverage_workflow)

    refute status.success?
    assert_includes stderr, "must appear under the expected roadmap section"
  end

  # Issue #211: README coverage badge binding evidence

  def test_accepts_readme_coverage_badge_bound_evidence
    repository_root = REPOSITORY_ROOT
    workflow = (repository_root / ".github/workflows/ci.yml").read
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      ## Current delivery status

      ### v0.1 acceptance checklist

      - [x] The README coverage badge percentage is bound to ci.yml's enforced --fail-under-lines and --fail-under-regions thresholds. <!-- roadmap-evidence: readme-coverage-badge-bound -->
    MARKDOWN

    stdout, stderr, status = run_checker(roadmap: roadmap, workflow: workflow)

    assert status.success?, stderr
    assert_includes stdout, "Roadmap evidence policy passed."
  end

  def test_rejects_readme_coverage_badge_bound_evidence_with_the_wrong_claim
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      ## Current delivery status

      ### v0.1 acceptance checklist

      - [x] The README coverage badge is green. <!-- roadmap-evidence: readme-coverage-badge-bound -->
    MARKDOWN

    _stdout, stderr, status = run_checker(roadmap: roadmap, workflow: coverage_workflow)

    refute status.success?
    assert_includes stderr, "does not prove this roadmap claim"
  end

  def test_rejects_readme_coverage_badge_bound_evidence_outside_the_v0_1_checklist
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      ## v1.0 — spec freeze

      ### v0.1 acceptance checklist

      - [x] The README coverage badge percentage is bound to ci.yml's enforced --fail-under-lines and --fail-under-regions thresholds. <!-- roadmap-evidence: readme-coverage-badge-bound -->
    MARKDOWN

    _stdout, stderr, status = run_checker(roadmap: roadmap, workflow: coverage_workflow)

    refute status.success?
    assert_includes stderr, "must appear under the expected roadmap section"
  end

  def test_accepts_throughput_floor_evidence
    repository_root = REPOSITORY_ROOT
    workflow = (repository_root / ".github/workflows/ci.yml").read
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      ## Current delivery status

      ### v0.1 acceptance checklist

      - [x] `pycc check` processes 1k LOC in under 75 ms. <!-- roadmap-evidence: check-throughput-1k-loc-75ms -->
    MARKDOWN

    stdout, stderr, status = run_checker(roadmap: roadmap, workflow: workflow)

    assert status.success?, stderr
    assert_includes stdout, "Roadmap evidence policy passed."
  end

  def test_rejects_throughput_floor_evidence_with_the_wrong_claim
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      ## Current delivery status

      ### v0.1 acceptance checklist

      - [x] `pycc check` processes 1k LOC in under 5 seconds. <!-- roadmap-evidence: check-throughput-1k-loc-75ms -->
    MARKDOWN

    _stdout, stderr, status = run_checker(roadmap: roadmap, workflow: coverage_workflow)

    refute status.success?
    assert_includes stderr, "does not prove this roadmap claim"
  end

  def test_rejects_throughput_floor_evidence_outside_the_v0_1_checklist
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      ## v1.0 — spec freeze

      ### v0.1 acceptance checklist

      - [x] `pycc check` processes 1k LOC in under 75 ms. <!-- roadmap-evidence: check-throughput-1k-loc-75ms -->
    MARKDOWN

    _stdout, stderr, status = run_checker(roadmap: roadmap, workflow: coverage_workflow)

    refute status.success?
    assert_includes stderr, "must appear under the expected roadmap section"
  end

  def test_accepts_cli_spec_diagnostic_evidence
    repository_root = REPOSITORY_ROOT
    workflow = (repository_root / ".github/workflows/ci.yml").read
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      ## Current delivery status

      ### v0.1 acceptance checklist

      - [x] The error demonstration matches the stable [CLI specification](./CLI_SPEC.md) output. <!-- roadmap-evidence: cli-spec-diagnostic-match -->
    MARKDOWN

    stdout, stderr, status = run_checker(roadmap: roadmap, workflow: workflow)

    assert status.success?, stderr
    assert_includes stdout, "Roadmap evidence policy passed."
  end

  def test_rejects_cli_spec_diagnostic_evidence_with_the_wrong_claim
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      ## Current delivery status

      ### v0.1 acceptance checklist

      - [x] The error demonstration matches CLI_SPEC.md. <!-- roadmap-evidence: cli-spec-diagnostic-match -->
    MARKDOWN

    _stdout, stderr, status = run_checker(roadmap: roadmap, workflow: coverage_workflow)

    refute status.success?
    assert_includes stderr, "does not prove this roadmap claim"
  end

  def test_rejects_cli_spec_diagnostic_evidence_outside_the_v0_1_checklist
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      ## v1.0 — spec freeze

      ### v0.1 acceptance checklist

      - [x] The error demonstration matches the stable [CLI specification](./CLI_SPEC.md) output. <!-- roadmap-evidence: cli-spec-diagnostic-match -->
    MARKDOWN

    _stdout, stderr, status = run_checker(roadmap: roadmap, workflow: coverage_workflow)

    refute status.success?
    assert_includes stderr, "must appear under the expected roadmap section"
  end

  # The checker activates before the workflow under D-103. Its exact self-test
  # therefore accepts the old D211 live digest and the reviewed 3.14.7
  # successor, but no third shape.
  def test_tier1_workflow_authorization_is_in_the_python_3147_transition
    live_digest = Digest::SHA256.hexdigest(
      (REPOSITORY_ROOT / ".github/workflows/ci.yml").read
    )

    assert_includes(
      [
        D211_COVERAGE_BADGE_BINDING_CI_WORKFLOW_SHA256,
        PY3147_ORACLE_CI_WORKFLOW_SHA256
      ],
      live_digest
    )
  end

  # D100/D112's own retirement (later, separate propose-then-activate
  # rounds for check_roadmap_evidence.rb's own bytes) will shrink this
  # array further -- until then all three coexist, mirroring D-090's own
  # coexist-then-retire precedent for this exact array. D199 is now the
  # live `.github/workflows/ci.yml` shape (see
  # test_tier1_workflow_authorization_is_the_active_d199_digest above);
  # D100/D112/D114/D229 remain accepted alongside it only as retained,
  # no-longer-live audit evidence.
  # Issue #229 (Phase 3 activation): the D229 pages-performance digest
  # is retained audit evidence.  Issue #199 (Merge 2 activation): the
  # D199 pages-accessibility digest is now the live ci.yml shape.
  def test_tier1_workflow_authorization_includes_staged_python_3147_oracle
    assert_equal(
      [
        D100_COMPOSE_D91_D99_CI_WORKFLOW_SHA256,
        D112_UBUNTU_FRONTEND_PERF_CI_WORKFLOW_SHA256,
        D114_FRONTEND_PERF_THRESHOLD_CI_WORKFLOW_SHA256,
        D229_PAGES_PERFORMANCE_CI_WORKFLOW_SHA256,
        D199_PAGES_ACCESSIBILITY_CI_WORKFLOW_SHA256,
        D211_COVERAGE_BADGE_BINDING_CI_WORKFLOW_SHA256,
        PY3147_ORACLE_CI_WORKFLOW_SHA256,
        D171_CHANGE_AWARE_CI_WORKFLOW_SHA256
      ],
      REVIEWED_PERF_CI_WORKFLOW_SHA256S
    )
  end

  # D-114 (round 5/6): this reviewed successor fixture -- identical to
  # the D112 shape except the gate job's own comparison step gains an
  # explicit "7.0" threshold_percent argument -- is now retained audit
  # evidence only. D199 is now the live
  # `.github/workflows/ci.yml` shape (see
  # test_tier1_workflow_authorization_is_the_active_d199_digest above);
  # D114 remains accepted alongside it only as retained, no-longer-live
  # audit evidence.
  def test_d114_frontend_perf_threshold_workflow_fixture_matches_its_own_digest
    assert_equal(
      D114_FRONTEND_PERF_THRESHOLD_CI_WORKFLOW_SHA256,
      Digest::SHA256.file(D114_FRONTEND_PERF_THRESHOLD_WORKFLOW_FIXTURE).hexdigest
    )
  end

  def test_d114_frontend_perf_threshold_workflow_is_now_active
    assert_includes REVIEWED_PERF_CI_WORKFLOW_SHA256S,
                    D114_FRONTEND_PERF_THRESHOLD_CI_WORKFLOW_SHA256
  end

  # Issue #229 (Phase 3 activation): D229 was the live ci.yml shape.
  # Issue #199 (Merge 2 activation): D199 is now the live ci.yml shape.
  # This test verifies the D229 pages-performance ci.yml fixture still
  # matches its own digest (retained audit evidence), and that the live
  # D199 ci.yml passes the lifecycle validators (source-aware perf gate
  # and pages-performance lifecycle).
  def test_d229_pages_performance_workflow_fixture_matches_its_own_digest
    assert_equal(
      D229_PAGES_PERFORMANCE_CI_WORKFLOW_SHA256,
      Digest::SHA256.file(D229_PAGES_PERFORMANCE_WORKFLOW_FIXTURE).hexdigest
    )
  end

  def test_python_3147_transition_workflow_is_active_and_reviewed
    assert_equal(
      D211_COVERAGE_BADGE_BINDING_CI_WORKFLOW_SHA256,
      Digest::SHA256.file(D211_COVERAGE_BADGE_BINDING_WORKFLOW_FIXTURE).hexdigest
    )
    live_digest = Digest::SHA256.file(ACTIVE_D100_COMPOSE_D91_D99_WORKFLOW).hexdigest
    expected_fixture =
      if live_digest == D211_COVERAGE_BADGE_BINDING_CI_WORKFLOW_SHA256
        D211_COVERAGE_BADGE_BINDING_WORKFLOW_FIXTURE
      else
        PY3147_ORACLE_WORKFLOW_FIXTURE
      end
    assert_includes(
      [
        D211_COVERAGE_BADGE_BINDING_CI_WORKFLOW_SHA256,
        PY3147_ORACLE_CI_WORKFLOW_SHA256
      ],
      live_digest
    )
    assert_equal expected_fixture.read, ACTIVE_D100_COMPOSE_D91_D99_WORKFLOW.read
    assert validate_source_aware_perf_gate_lifecycle(
      ACTIVE_D100_COMPOSE_D91_D99_WORKFLOW.read,
      ACTIVE_D100_COMPOSE_D91_D99_WORKFLOW.to_s
    )
    assert coverage_gate_present?(
      ACTIVE_D100_COMPOSE_D91_D99_WORKFLOW.read,
      ACTIVE_D100_COMPOSE_D91_D99_WORKFLOW.to_s
    )
    assert validate_pages_performance_lifecycle(
      ACTIVE_D100_COMPOSE_D91_D99_WORKFLOW.read,
      ACTIVE_D100_COMPOSE_D91_D99_WORKFLOW.to_s
    )
  end

  def test_d114_frontend_perf_threshold_workflow_structure_is_recognized
    workflow_text = D114_FRONTEND_PERF_THRESHOLD_WORKFLOW_FIXTURE.read
    assert validate_source_aware_perf_gate_lifecycle(
      workflow_text, D114_FRONTEND_PERF_THRESHOLD_WORKFLOW_FIXTURE.to_s
    )
  end

  def test_d114_frontend_perf_threshold_workflow_compare_step_has_the_new_argument
    workflow = Psych.safe_load(D114_FRONTEND_PERF_THRESHOLD_WORKFLOW_FIXTURE.read)
    steps = workflow.dig("jobs", "frontend-perf-gate", "steps")
    compare = steps.find { |step| step["name"] == "Compare exact predecessor and candidate" }
    assert_includes compare["run"], "\"7.0\""
  end

  # v0.2 PR-8's own merge is the activation commit for the coverage-step
  # shape too (same as the digest array above): `ci.yml` now matches the
  # D91 coverage-step shape exclusively (D-099's own vcpkg-cache change never
  # touched build-test-coverage's own step at all), so the pre-D91 shape
  # (`COVERAGE_SCRIPT`/`TRUSTED_COVERAGE_STEPS`) must no longer be accepted
  # alongside it.
  def test_coverage_gate_authorization_contains_only_active_d91
    assert_equal([D91_COVERAGE_SCRIPT], REVIEWED_COVERAGE_SCRIPTS)
    assert_equal([D91_TRUSTED_COVERAGE_STEPS], REVIEWED_TRUSTED_COVERAGE_STEPS)
  end

  # D-090's own fixture is gone: it was staged but never activated, and
  # was found (while opening PR-8's own pull request) to be missing the
  # coverage-sandbox release build D-091 adds -- see D-091's comment in
  # check_roadmap_evidence.rb for the full correction. Its digest constant
  # remains only as a historical record that it was once reviewed and
  # staged, matching D51/D56/D62/D80's own "no longer accepted" pattern.

  # Retained pre-D100 audit fixture: D91 is D84 plus PR-8's release-mode
  # runtime/coverage builds and relaxed manifest contract. D-099 activated
  # first on `main` (unrelated to PR-8), retiring this digest before D-100
  # composed it back in alongside D-099's own cache boundary.
  def test_d91_relax_frontend_perf_manifest_workflow_remains_an_audit_fixture
    assert_equal(
      D91_RELAX_FRONTEND_PERF_MANIFEST_CI_WORKFLOW_SHA256,
      Digest::SHA256.file(D91_RELAX_FRONTEND_PERF_MANIFEST_WORKFLOW_FIXTURE).hexdigest
    )
    refute_includes REVIEWED_PERF_CI_WORKFLOW_SHA256S,
                    D91_RELAX_FRONTEND_PERF_MANIFEST_CI_WORKFLOW_SHA256
  end


  # `coverage_gate_present?`/`D91_COVERAGE_SCRIPT` model the exact body of
  # D91's "Hard coverage gate" step, unlike the frontend-perf-measure job the
  # lifecycle validator above checks. Keeping this pre-D99 audit fixture
  # structurally recognized proves it remains a sound input for a future
  # D91+D99 composition; recognition alone does not publicly authorize its
  # whole-file digest or permit direct activation.
  def test_d91_relax_frontend_perf_manifest_workflow_still_has_a_recognized_coverage_gate
    assert coverage_gate_present?(
      D91_RELAX_FRONTEND_PERF_MANIFEST_WORKFLOW_FIXTURE.read,
      D91_RELAX_FRONTEND_PERF_MANIFEST_WORKFLOW_FIXTURE.to_s
    )
  end

  # Retained pre-D100 audit fixture: D99 (D84 plus the Windows vcpkg cache
  # boundary) activated on `main` before this PR-8 branch's own merge, but
  # is itself retired once D-100 composes it with D-091's changes. Its own
  # coverage step is still D84's pre-D91 shape (D-099 never touched
  # `build-test-coverage` at all), so it does NOT satisfy the now-narrowed
  # `coverage_gate_present?` -- matching D91's own retired-fixture pattern
  # above, not the pre-narrowing expectation an earlier draft of this test
  # assumed.
  def test_d99_vcpkg_libxml2_cache_workflow_digest_matches_the_reviewed_fixture
    assert_equal(
      D99_VCPKG_LIBXML2_CACHE_CI_WORKFLOW_SHA256,
      Digest::SHA256.file(D99_VCPKG_LIBXML2_CACHE_WORKFLOW_FIXTURE).hexdigest
    )
    refute_includes REVIEWED_PERF_CI_WORKFLOW_SHA256S,
                    D99_VCPKG_LIBXML2_CACHE_CI_WORKFLOW_SHA256
    assert validate_source_aware_perf_gate_lifecycle(
      D99_VCPKG_LIBXML2_CACHE_WORKFLOW_FIXTURE.read,
      D99_VCPKG_LIBXML2_CACHE_WORKFLOW_FIXTURE.to_s
    )
    refute coverage_gate_present?(
      D99_VCPKG_LIBXML2_CACHE_WORKFLOW_FIXTURE.read,
      D99_VCPKG_LIBXML2_CACHE_WORKFLOW_FIXTURE.to_s
    )
  end

  def test_d99_changes_only_the_retired_d84_native_job_cache_steps
    retired = Psych.load(D84_THROUGHPUT_FLOOR_WORKFLOW_FIXTURE.read)
    active = Psych.load(D99_VCPKG_LIBXML2_CACHE_WORKFLOW_FIXTURE.read)
    active.fetch("jobs")
          .fetch("native-build-test")
          .fetch("steps")
          .reject! do |step|
            [
              "Configure vcpkg binary cache identity (Windows)",
              "Restore vcpkg libxml2 binary cache (Windows)",
              "Save vcpkg libxml2 binary cache (Windows main only)"
            ].include?(step["name"])
          end

    assert_equal retired, active
  end

  def test_d99_vcpkg_cache_key_and_write_boundary_are_fail_closed
    steps = Psych.load(D99_VCPKG_LIBXML2_CACHE_WORKFLOW_FIXTURE.read)
                 .fetch("jobs")
                 .fetch("native-build-test")
                 .fetch("steps")
    identity = steps.find do |step|
      step["name"] == "Configure vcpkg binary cache identity (Windows)"
    end
    restore = steps.find do |step|
      step["name"] == "Restore vcpkg libxml2 binary cache (Windows)"
    end
    install = steps.find do |step|
      step["name"] == "Install libxml2 (Windows, via vcpkg, for llvm-sys's system-libs)"
    end
    save = steps.find do |step|
      step["name"] == "Save vcpkg libxml2 binary cache (Windows main only)"
    end

    refute_nil identity
    refute_nil restore
    refute_nil install
    refute_nil save
    assert_operator steps.index(identity), :<, steps.index(restore)
    assert_operator steps.index(restore), :<, steps.index(install)
    assert_operator steps.index(install), :<, steps.index(save)

    expected_action =
      "actions/cache/%<mode>s@55cc8345863c7cc4c66a329aec7e433d2d1c52a9"
    assert_equal format(expected_action, mode: "restore"), restore.fetch("uses")
    assert_equal format(expected_action, mode: "save"), save.fetch("uses")
    assert_equal "runner.os == 'Windows'", identity.fetch("if")
    assert_equal "runner.os == 'Windows'", restore.fetch("if")
    assert_includes identity.fetch("run"),
                    "git -C $env:VCPKG_INSTALLATION_ROOT rev-parse HEAD"
    assert_includes identity.fetch("run"), "'^[0-9a-f]{40}$'"
    assert_includes identity.fetch("run"), "$imageVersion = $env:ImageVersion"
    assert_includes identity.fetch("run"),
                    "'^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$'"
    assert_includes identity.fetch("run"),
                    "\"image_version=$imageVersion\" >> $env:GITHUB_OUTPUT"
    assert_includes identity.fetch("run"),
                    "\"VCPKG_DEFAULT_BINARY_CACHE=$cacheDir\" >> $env:GITHUB_ENV"

    expected_key =
      "vcpkg-libxml2-${{ runner.os }}-${{ runner.arch }}-" \
      "image-${{ steps.vcpkg_cache_identity.outputs.image_version }}-" \
      "llvm-${{ env.LLVM_VERSION }}-x64-windows-static-md-" \
      "${{ steps.vcpkg_cache_identity.outputs.vcpkg_commit }}"
    assert_equal expected_key, restore.fetch("with").fetch("key")
    refute restore.fetch("with").key?("restore-keys")
    assert_equal(
      "${{ steps.vcpkg_cache_identity.outputs.cache_path }}",
      restore.fetch("with").fetch("path")
    )
    assert_equal(
      "runner.os == 'Windows' && github.event_name == 'push' && " \
      "github.ref == 'refs/heads/main' && " \
      "steps.vcpkg_libxml2_cache_restore.outputs.cache-hit != 'true'",
      save.fetch("if")
    )
    assert_equal(
      "${{ steps.vcpkg_libxml2_cache_restore.outputs.cache-primary-key }}",
      save.fetch("with").fetch("key")
    )
    assert_equal restore.fetch("with").fetch("path"),
                 save.fetch("with").fetch("path")
  end

  # D-091: the bench-manifest fingerprint (`[dev-dependencies]` onward) must
  # hard-abort on a change to the bench-defining tail itself -- otherwise a
  # PR that only speeds up `criterion`/`[[bench]]` could pass the perf gate
  # as though it sped up the compiler. Confirmed by executing the real
  # script, not by reading its source.
  def test_d91_bench_manifest_fingerprint_hard_aborts_on_bench_tooling_change
    _stdout, stderr, status = run_d91_verify_revisions do |root|
      (root / "previous/Cargo.toml").write(d91_cargo_toml)
      (root / "current/Cargo.toml").write(
        d91_cargo_toml(
          bench_manifest_tail: D91_BENCH_MANIFEST_TAIL.sub("0.8.2", "0.9.0")
        )
      )
    end
    refute status.success?, "expected a criterion version bump to hard-abort, got: #{stderr}"
  end

  # D-091: an ordinary product dependency addition (PR-8's own toml/serde
  # shape) must NOT trip the bench-manifest fingerprint -- only the
  # `[dev-dependencies]`-onward tail is hard-required identical.
  def test_d91_bench_manifest_fingerprint_allows_product_dependency_only_change
    _stdout, stderr, status = run_d91_verify_revisions do |root|
      (root / "previous/Cargo.toml").write(d91_cargo_toml)
      (root / "current/Cargo.toml").write(
        d91_cargo_toml(dependencies_extra: %(toml = "0.8"\nserde = "1"))
      )
    end
    assert status.success?, stderr
  end

  # D-091: the fingerprint's own invariant guard must fail loudly, not
  # silently mis-scope, if a manifest is missing `[dev-dependencies]`
  # entirely.
  def test_d91_bench_manifest_fingerprint_hard_aborts_when_dev_dependencies_is_missing
    _stdout, stderr, status = run_d91_verify_revisions do |root|
      (root / "previous/Cargo.toml").write(d91_cargo_toml)
      (root / "current/Cargo.toml").write(<<~TOML)
        [package]
        name = "pycc"
        version = "0.1.0"

        [dependencies]
        clap = { version = "4", features = ["derive"] }
      TOML
    end
    refute status.success?
    assert_includes stderr, "bench-manifest fingerprint invariant violated"
  end

  # D-091: the guard must also fail loudly if a future manifest reorders
  # sections so something other than `[[bench]]` follows
  # `[dev-dependencies]`, rather than silently widening the hard-required
  # region to swallow an otherwise-reclassifiable dependency.
  def test_d91_bench_manifest_fingerprint_hard_aborts_on_unexpected_trailing_section
    _stdout, stderr, status = run_d91_verify_revisions do |root|
      (root / "previous/Cargo.toml").write(d91_cargo_toml)
      (root / "current/Cargo.toml").write(
        d91_cargo_toml(bench_manifest_tail: "#{D91_BENCH_MANIFEST_TAIL}\n[extra]\nfoo = 1\n")
      )
    end
    refute status.success?
    assert_includes stderr, "unexpected section"
  end

  # D-091: the guard must also fail loudly if `[[bench]]` is reordered to
  # appear ABOVE `[dev-dependencies]` instead of after it -- otherwise the
  # `[dev-dependencies]`-onward extraction would no longer include
  # `[[bench]]` at all, silently moving it out of the hard-pinned tail and
  # into the softer reclassification, reopening the exact P1 hole this
  # fingerprint exists to close.
  def test_d91_bench_manifest_fingerprint_hard_aborts_when_bench_precedes_dev_dependencies
    reordered = <<~TOML
      [package]
      name = "pycc"
      version = "0.1.0"

      [dependencies]
      clap = { version = "4", features = ["derive"] }

      [[bench]]
      name = "check_bench"
      harness = false

      [dev-dependencies]
      serde_json = "1"
      criterion = { version = "0.8.2", features = ["html_reports"] }
    TOML
    _stdout, stderr, status = run_d91_verify_revisions do |root|
      (root / "previous/Cargo.toml").write(d91_cargo_toml)
      (root / "current/Cargo.toml").write(reordered)
    end
    refute status.success?
    assert_includes stderr, "outside its [dev-dependencies]-onward tail"
  end

  # D-091: root-level build.rs must be classified (Cargo would silently use
  # it without any Cargo.toml change), while an unrelated identical src/
  # tree still reports executable_inputs_equal=true.
  def test_d91_classifier_reports_identical_and_added_build_rs
    _stdout, stderr, status, output = run_d91_executable_input_classifier do |root|
      (root / "previous/src/lib.rs").write("same\n")
      (root / "current/src/lib.rs").write("same\n")
    end
    assert status.success?, stderr
    assert_equal "executable_inputs_equal=true\n", output

    _stdout, stderr, status, output = run_d91_executable_input_classifier do |root|
      (root / "previous/src/lib.rs").write("same\n")
      (root / "current/src/lib.rs").write("same\n")
      (root / "current/build.rs").write("fn main() {}\n")
    end
    assert status.success?, stderr
    assert_equal "executable_inputs_equal=false\n", output
  end

  def test_tier1_workflow_allowlist_retires_the_pre_alpha_eval_digest
    refute_equal(
      D80_CONFORMANCE_ORACLE_CI_WORKFLOW_SHA256,
      "58e2d5026b59e7c921b57c882d24b6507c95dd8f99e390c0a68af217e5e038c8"
    )
  end

  def test_d80_conformance_oracle_digest_is_retained_as_historical_audit_evidence
    assert_equal(
      "17611d861d10c34d6ccebbf21bc82d8dfaf006b969bb2fe1e12d57b9e9c81234",
      D80_CONFORMANCE_ORACLE_CI_WORKFLOW_SHA256
    )
    refute_includes REVIEWED_PERF_CI_WORKFLOW_SHA256S,
                    D80_CONFORMANCE_ORACLE_CI_WORKFLOW_SHA256
  end

  def test_tier1_workflow_allowlist_retires_the_d48_steady_digest
    refute_equal(
      D80_CONFORMANCE_ORACLE_CI_WORKFLOW_SHA256,
      "940b342845a9fc600d72195a0a382ce9437f3cb123cc62f8805b8cb82ae35f56"
    )
  end

  def test_tier1_workflow_allowlist_retires_the_d62_replicated_digest
    refute_equal(
      D80_CONFORMANCE_ORACLE_CI_WORKFLOW_SHA256,
      D62_REPLICATED_SOURCE_AWARE_PERF_CI_WORKFLOW_SHA256
    )
  end

  def test_retired_d51_paired_workflow_remains_a_reviewed_audit_fixture
    assert_equal(
      D51_PAIRED_PERF_CI_WORKFLOW_SHA256,
      Digest::SHA256.file(RETIRED_D51_PAIRED_WORKFLOW).hexdigest
    )
    assert validate_perf_gate_baseline_lifecycle(
      RETIRED_D51_PAIRED_WORKFLOW.read,
      RETIRED_D51_PAIRED_WORKFLOW.to_s
    )
  end

  def test_d56_source_aware_workflow_remains_reviewed_audit_evidence
    assert D56_SOURCE_AWARE_WORKFLOW_FIXTURE.file?
    assert_equal(
      D56_SOURCE_AWARE_PERF_CI_WORKFLOW_SHA256,
      Digest::SHA256.file(D56_SOURCE_AWARE_WORKFLOW_FIXTURE).hexdigest
    )
    assert validate_source_aware_perf_gate_lifecycle(
      D56_SOURCE_AWARE_WORKFLOW_FIXTURE.read,
      D56_SOURCE_AWARE_WORKFLOW_FIXTURE.to_s
    )
    refute_equal(
      D51_PAIRED_PERF_CI_WORKFLOW_SHA256,
      D56_SOURCE_AWARE_PERF_CI_WORKFLOW_SHA256
    )
    assert_equal(
      D56_PERF_CHECKER_SHA256,
      Digest::SHA256.file(
        REPOSITORY_ROOT / "scripts/check_source_aware_perf_regression.rb"
      ).hexdigest
    )
    assert_equal(
      D56_PERF_CHECKER_TEST_SHA256,
      Digest::SHA256.file(
        REPOSITORY_ROOT / "scripts/test_check_source_aware_perf_regression.rb"
      ).hexdigest
    )
  end

  def test_d56_classifier_reports_identical_and_changed_executable_inputs
    _stdout, stderr, status, output = run_executable_input_classifier do |root|
      (root / "previous/src/lib.rs").write("same\n")
      (root / "current/src/lib.rs").write("same\n")
    end
    assert status.success?, stderr
    assert_equal "executable_inputs_equal=true\n", output

    _stdout, stderr, status, output = run_executable_input_classifier do |root|
      (root / "previous/crates/lib.rs").write("old\n")
      (root / "current/crates/lib.rs").write("new\n")
    end
    assert status.success?, stderr
    assert_equal "executable_inputs_equal=false\n", output
  end

  def test_d56_gate_accepts_only_a_boolean_executable_input_identity
    %w[true false].each do |value|
      _stdout, stderr, status =
        run_executable_input_identity_requirement(value)
      assert status.success?, stderr
    end

    ["", "unknown", "TRUE", "0"].each do |value|
      _stdout, stderr, status =
        run_executable_input_identity_requirement(value)
      refute status.success?
      assert_includes stderr, "invalid executable-input identity"
    end
  end

  def test_d56_rejects_missing_executable_input_output
    workflow = source_aware_perf_workflow do |jobs|
      jobs.fetch("frontend-perf-measure").fetch("outputs").delete(
        "executable_inputs_equal"
      )
    end

    error = assert_raises(RoadmapEvidenceError) do
      validate_source_aware_perf_gate_lifecycle(workflow, "ci.yml")
    end
    assert_includes error.message, "reviewed source-aware measurement job"
  end

  def test_d56_rejects_classifier_after_candidate_execution
    workflow = source_aware_perf_workflow do |jobs|
      steps = jobs.fetch("frontend-perf-measure").fetch("steps")
      classifier_index = steps.index do |step|
        step["name"] == "Classify executable benchmark inputs"
      end
      classifier = steps.delete_at(classifier_index)
      candidate_index = steps.index do |step|
        step["name"] == "Benchmark exact candidate"
      end
      steps.insert(candidate_index + 1, classifier)
    end

    error = assert_raises(RoadmapEvidenceError) do
      validate_source_aware_perf_gate_lifecycle(workflow, "ci.yml")
    end
    assert_includes error.message, "reviewed source-aware measurement job"
  end

  def test_d56_rejects_an_incomplete_executable_input_set
    %w[src crates].each do |path|
      workflow = source_aware_perf_workflow do |jobs|
        classifier = jobs.fetch("frontend-perf-measure").fetch("steps").find do |step|
          step["name"] == "Classify executable benchmark inputs"
        end
        classifier["run"] = classifier.fetch("run").sub(
          "for executable_path in src crates",
          "for executable_path in #{path}"
        )
      end

      error = assert_raises(RoadmapEvidenceError) do
        validate_source_aware_perf_gate_lifecycle(workflow, "ci.yml")
      end
      assert_includes error.message, "reviewed source-aware measurement job"
    end
  end

  def test_d56_rejects_gate_without_identity_validation_or_comparator_binding
    ["Require executable-input identity", "Compare exact predecessor and candidate"].each do |step_name|
      workflow = source_aware_perf_workflow do |jobs|
        steps = jobs.fetch("frontend-perf-gate").fetch("steps")
        step = steps.find { |candidate| candidate["name"] == step_name }
        step.delete("env")
      end

      error = assert_raises(RoadmapEvidenceError) do
        validate_source_aware_perf_gate_lifecycle(workflow, "ci.yml")
      end
      assert_includes error.message, "reviewed source-aware comparison job"
    end
  end

  def test_d62_replicated_workflow_remains_a_reviewed_audit_fixture
    assert_equal(
      D62_REPLICATED_SOURCE_AWARE_PERF_CI_WORKFLOW_SHA256,
      Digest::SHA256.file(D62_REPLICATED_PAIRED_WORKFLOW_FIXTURE).hexdigest
    )
    assert validate_source_aware_perf_gate_lifecycle(
      D62_REPLICATED_PAIRED_WORKFLOW_FIXTURE.read,
      D62_REPLICATED_PAIRED_WORKFLOW_FIXTURE.to_s
    )
  end

  # D100's own digest remains in REVIEWED_PERF_CI_WORKFLOW_SHA256S (see
  # the coexist test above), but the live ci.yml file itself now matches
  # D229, not D100 (see
  # test_tier1_workflow_authorization_is_the_active_d199_digest above) --
  # D100's retirement from the checker's array is a separate, later round.
  def test_d100_composed_workflow_fixture_matches_its_own_digest
    assert_equal(
      D100_COMPOSE_D91_D99_CI_WORKFLOW_SHA256,
      Digest::SHA256.file(D100_COMPOSED_WORKFLOW_FIXTURE).hexdigest
    )
    assert_equal(
      REPLICATED_PERF_CHECKER_SHA256,
      Digest::SHA256.file(
        REPOSITORY_ROOT /
          "scripts/check_replicated_paired_perf_regression.rb"
      ).hexdigest
    )
    assert_equal(
      REPLICATED_PERF_CHECKER_TEST_SHA256,
      Digest::SHA256.file(
        REPOSITORY_ROOT /
          "scripts/test_check_replicated_paired_perf_regression.rb"
      ).hexdigest
    )
    assert validate_source_aware_perf_gate_lifecycle(
      D100_COMPOSED_WORKFLOW_FIXTURE.read,
      D100_COMPOSED_WORKFLOW_FIXTURE.to_s
    )
    assert coverage_gate_present?(
      D100_COMPOSED_WORKFLOW_FIXTURE.read,
      D100_COMPOSED_WORKFLOW_FIXTURE.to_s
    )
  end

  # D-112 is no longer the live shape once D199 activates (see
  # test_d199_pages_accessibility_workflow_is_active_and_reviewed
  # above) -- its remaining, still-true facts (fixture digest,
  # array membership, structural recognition) stay covered by
  # test_d112_ubuntu_frontend_perf_workflow_digest_matches_the_fixture,
  # test_d112_ubuntu_frontend_perf_workflow_is_now_active, and
  # test_d112_ubuntu_frontend_perf_workflow_structure_is_recognized
  # below, so this test (which asserted D112 == the live file) is
  # removed rather than left asserting something now false.

  # D112 is no longer the live shape once D199 activates (see
  # test_d199_pages_accessibility_workflow_is_active_and_reviewed
  # above), but the checker still recognizes and accepts it (this fixture
  # is identical to the live D91/REPLICATED frontend-perf-measure/gate
  # shape except runs-on: ubuntu-latest and the macOS brew-based LLVM
  # install swapped for native-build-test's own already-reviewed
  # apt.llvm.org Linux install step) -- retained as reviewed audit
  # evidence per D-112's own Consequences in docs/DECISIONS.md.
  def test_d112_ubuntu_frontend_perf_workflow_digest_matches_the_fixture
    assert_equal(
      D112_UBUNTU_FRONTEND_PERF_CI_WORKFLOW_SHA256,
      Digest::SHA256.file(D112_UBUNTU_FRONTEND_PERF_WORKFLOW_FIXTURE).hexdigest
    )
  end

  def test_d112_ubuntu_frontend_perf_workflow_is_now_active
    assert_includes REVIEWED_PERF_CI_WORKFLOW_SHA256S,
                    D112_UBUNTU_FRONTEND_PERF_CI_WORKFLOW_SHA256
  end

  def test_d112_ubuntu_frontend_perf_workflow_structure_is_recognized
    workflow_text = D112_UBUNTU_FRONTEND_PERF_WORKFLOW_FIXTURE.read
    assert validate_source_aware_perf_gate_lifecycle(
      workflow_text, D112_UBUNTU_FRONTEND_PERF_WORKFLOW_FIXTURE.to_s
    )
  end

  def test_rejects_d112_measurement_job_with_a_different_runner
    workflow = d112_ubuntu_frontend_perf_workflow do |jobs|
      jobs.fetch("frontend-perf-measure")["runs-on"] = "ubuntu-24.04" # plausible near-miss, not the pinned value
    end

    error = assert_raises(RoadmapEvidenceError) do
      validate_source_aware_perf_gate_lifecycle(workflow, "ci.yml")
    end
    assert_includes error.message, "reviewed source-aware measurement job"
  end

  # D-114: the same D112_UBUNTU_FRONTEND_PERF_MEASURE_JOB measure job
  # authorizes either gate-job shape (D-112's 2.0%-implicit-threshold shape,
  # still covered by the tests above, or this 7.0%-explicit-threshold
  # shape, now retained audit evidence -- see
  # test_tier1_workflow_authorization_is_the_active_d199_digest above).
  def test_d114_raised_threshold_frontend_perf_gate_job_structure_is_recognized
    workflow = d114_raised_threshold_frontend_perf_workflow
    assert validate_source_aware_perf_gate_lifecycle(workflow, "ci.yml")
  end

  def test_d114_raised_threshold_frontend_perf_gate_job_passes_the_raised_threshold
    compare_step =
      D114_RAISED_THRESHOLD_FRONTEND_PERF_GATE_JOB
      .fetch("steps")
      .find { |step| step["name"] == "Compare exact predecessor and candidate" }
    assert_includes compare_step.fetch("run"), '"7.0"'
  end

  def test_rejects_d114_measurement_job_with_a_different_runner
    workflow = d114_raised_threshold_frontend_perf_workflow do |jobs|
      jobs.fetch("frontend-perf-measure")["runs-on"] = "ubuntu-24.04" # plausible near-miss, not the pinned value
    end

    error = assert_raises(RoadmapEvidenceError) do
      validate_source_aware_perf_gate_lifecycle(workflow, "ci.yml")
    end
    assert_includes error.message, "reviewed source-aware measurement job"
  end

  def test_rejects_d112_measurement_job_paired_with_an_unreviewed_gate_job_shape
    workflow = d112_ubuntu_frontend_perf_workflow do |jobs|
      jobs.fetch("frontend-perf-gate")["runs-on"] = "macos-14" # neither accepted gate-job shape
    end

    error = assert_raises(RoadmapEvidenceError) do
      validate_source_aware_perf_gate_lifecycle(workflow, "ci.yml")
    end
    assert_includes error.message, "reviewed source-aware comparison job"
  end

  # D-114's array-membership widening is scoped to the D112 measure-job
  # branch only -- a different measure job (REPLICATED_PERF_MEASURE_JOB
  # here) must still reject D114's gate-job shape, proving the new
  # permissiveness didn't leak across branches.
  def test_rejects_replicated_measurement_job_paired_with_the_d114_gate_job_shape
    workflow = replicated_perf_workflow do |jobs|
      jobs["frontend-perf-gate"] =
        Marshal.load(Marshal.dump(D114_RAISED_THRESHOLD_FRONTEND_PERF_GATE_JOB))
    end

    error = assert_raises(RoadmapEvidenceError) do
      validate_source_aware_perf_gate_lifecycle(workflow, "ci.yml")
    end
    assert_includes error.message, "reviewed source-aware comparison job"
  end

  # Issue #229 (Phase 2): validate_source_aware_perf_gate_lifecycle must
  # accept either the pre-D229 six-element ci-gate shape
  # (PAIRED_PERF_CI_GATE_JOB, without pages-performance) or the D229
  # seven-element ci-gate shape (D229_PAIRED_PERF_CI_GATE_JOB, with
  # pages-performance in needs and in the fail condition).  PR 5 adds
  # pages-performance to ci-gate.needs, so the validator must recognize
  # that new shape the moment it activates -- without retiring the old
  # shape before that activation lands.

  def test_d229_paired_perf_ci_gate_job_has_pages_performance_in_needs
    assert_includes D229_PAIRED_PERF_CI_GATE_JOB.fetch("needs"),
                    "pages-performance"
  end

  def test_d229_paired_perf_ci_gate_job_has_pages_performance_in_fail_condition
    fail_step =
      D229_PAIRED_PERF_CI_GATE_JOB.fetch("steps").find do |step|
        step["name"] == "Fail unless every required job succeeded"
      end
    assert_includes fail_step.fetch("if"),
                    "needs.pages-performance.result != 'success'"
  end

  def test_d229_paired_perf_ci_gate_job_has_seven_needs_entries
    assert_equal 7, D229_PAIRED_PERF_CI_GATE_JOB.fetch("needs").length
  end

  # Issue #199: the D199 eight-element ci-gate shape (with
  # pages-accessibility) must be accepted by
  # validate_source_aware_perf_gate_lifecycle.  This shape coexists
  # with the pre-D229 and D229 shapes until a later round retires
  # them.

  def test_d199_pages_accessibility_ci_gate_job_has_pages_accessibility_in_needs
    assert_includes D199_PAGES_ACCESSIBILITY_CI_GATE_JOB.fetch("needs"),
                    "pages-accessibility"
  end

  def test_d199_pages_accessibility_ci_gate_job_has_pages_accessibility_in_fail_condition
    fail_step =
      D199_PAGES_ACCESSIBILITY_CI_GATE_JOB.fetch("steps").find do |step|
        step["name"] == "Fail unless every required job succeeded"
      end
    assert_includes fail_step.fetch("if"),
                    "needs.pages-accessibility.result != 'success'"
  end

  def test_d199_pages_accessibility_ci_gate_job_has_eight_needs_entries
    assert_equal 8, D199_PAGES_ACCESSIBILITY_CI_GATE_JOB.fetch("needs").length
  end

  def test_d199_pages_accessibility_ci_gate_job_includes_pages_performance
    assert_includes D199_PAGES_ACCESSIBILITY_CI_GATE_JOB.fetch("needs"),
                    "pages-performance"
  end

  def test_source_aware_perf_gate_lifecycle_accepts_the_d199_ci_gate_shape
    workflow = d114_raised_threshold_frontend_perf_workflow do |jobs|
      jobs["ci-gate"] =
        Marshal.load(Marshal.dump(D199_PAGES_ACCESSIBILITY_CI_GATE_JOB))
    end
    assert validate_source_aware_perf_gate_lifecycle(workflow, "ci.yml")
  end

  def test_source_aware_perf_gate_lifecycle_rejects_d199_shape_without_pages_accessibility_fail_check
    # validate_pages_performance_lifecycle must reject a D199-shaped
    # ci-gate where the pages-accessibility fail check is missing.
    # We build a workflow with a valid pages-performance job and a
    # ci-gate that has pages-accessibility in needs but not in the
    # fail condition.
    gate = Marshal.load(Marshal.dump(D199_PAGES_ACCESSIBILITY_CI_GATE_JOB))
    fail_step = gate.fetch("steps").find { |s| s["name"] =~ /Fail unless/ }
    fail_step["if"] = fail_step["if"].sub(
      " || needs.pages-accessibility.result != 'success'", ""
    )
    workflow = pages_perf_workflow_yaml(
      pages_job: pages_perf_job_yaml,
      ci_gate: { "ci-gate" => gate }.to_yaml
    )

    error = assert_raises(RoadmapEvidenceError) do
      validate_pages_performance_lifecycle(workflow, "ci.yml")
    end
    assert_includes error.message,
                    "ci-gate fail step must check needs.pages-accessibility.result"
  end

  def test_source_aware_perf_gate_lifecycle_rejects_d199_shape_with_extra_needs_entry
    workflow = d114_raised_threshold_frontend_perf_workflow do |jobs|
      gate = Marshal.load(Marshal.dump(D199_PAGES_ACCESSIBILITY_CI_GATE_JOB))
      gate["needs"] = gate["needs"] + ["rogue-job"]
      jobs["ci-gate"] = gate
    end

    error = assert_raises(RoadmapEvidenceError) do
      validate_source_aware_perf_gate_lifecycle(workflow, "ci.yml")
    end
    assert_includes error.message, "reviewed fail-closed aggregate job"
  end

  def test_source_aware_perf_gate_lifecycle_accepts_the_pre_d229_ci_gate_shape
    workflow = d114_raised_threshold_frontend_perf_workflow
    assert validate_source_aware_perf_gate_lifecycle(workflow, "ci.yml")
  end

  def test_source_aware_perf_gate_lifecycle_accepts_the_d229_ci_gate_shape
    workflow = d114_raised_threshold_frontend_perf_workflow do |jobs|
      jobs["ci-gate"] =
        Marshal.load(Marshal.dump(D229_PAIRED_PERF_CI_GATE_JOB))
    end
    assert validate_source_aware_perf_gate_lifecycle(workflow, "ci.yml")
  end

  def test_source_aware_perf_gate_lifecycle_rejects_an_unreviewed_ci_gate_shape
    workflow = d114_raised_threshold_frontend_perf_workflow do |jobs|
      jobs["ci-gate"] =
        Marshal.load(Marshal.dump(D229_PAIRED_PERF_CI_GATE_JOB))
      jobs["ci-gate"]["runs-on"] = "macos-14" # neither accepted ci-gate shape
    end

    error = assert_raises(RoadmapEvidenceError) do
      validate_source_aware_perf_gate_lifecycle(workflow, "ci.yml")
    end
    assert_includes error.message, "reviewed fail-closed aggregate job"
  end

  def test_d84_throughput_floor_workflow_remains_a_retired_audit_fixture
    assert_equal(
      D84_THROUGHPUT_FLOOR_CI_WORKFLOW_SHA256,
      Digest::SHA256.file(D84_THROUGHPUT_FLOOR_WORKFLOW_FIXTURE).hexdigest
    )
    refute_includes REVIEWED_PERF_CI_WORKFLOW_SHA256S,
                    D84_THROUGHPUT_FLOOR_CI_WORKFLOW_SHA256
    assert validate_source_aware_perf_gate_lifecycle(
      D84_THROUGHPUT_FLOOR_WORKFLOW_FIXTURE.read,
      D84_THROUGHPUT_FLOOR_WORKFLOW_FIXTURE.to_s
    )
  end

  def test_d80_conformance_oracle_workflow_remains_a_reviewed_audit_fixture
    assert_equal(
      D80_CONFORMANCE_ORACLE_CI_WORKFLOW_SHA256,
      Digest::SHA256.file(D80_CONFORMANCE_ORACLE_WORKFLOW_FIXTURE).hexdigest
    )
    assert validate_source_aware_perf_gate_lifecycle(
      D80_CONFORMANCE_ORACLE_WORKFLOW_FIXTURE.read,
      D80_CONFORMANCE_ORACLE_WORKFLOW_FIXTURE.to_s
    )
  end

  def test_tier1_workflow_allowlist_retires_the_superseded_single_job_digest
    refute_equal(
      D62_REPLICATED_SOURCE_AWARE_PERF_CI_WORKFLOW_SHA256,
      "0079c33c46c085277c4a84996a69a6c2d1777b34de9daf2e5d5e8f1923ceb27c"
    )
  end

  def test_accepts_the_reviewed_paired_perf_trust_boundary
    assert validate_perf_gate_baseline_lifecycle(
      paired_perf_workflow,
      "ci.yml"
    )
  end

  def test_accepts_the_reviewed_fixed_replicate_perf_trust_boundary
    assert validate_source_aware_perf_gate_lifecycle(
      replicated_perf_workflow,
      "ci.yml"
    )
  end

  def test_public_cli_accepts_the_active_d100_workflow
    stdout, stderr, status = run_checker(
      roadmap: roadmap_with_tier1_claim(:absent),
      workflow: D100_COMPOSED_WORKFLOW_FIXTURE.read
    )

    assert status.success?, stderr
    assert_includes stdout, "Roadmap evidence policy passed."
  end

  # D99's own fixture is D84 plus the vcpkg-cache steps -- it predates D91's
  # coverage-step release-`pycc_rt`-build line, so it fails the earlier
  # `coverage_gate_present?` check (see the D56/D51/D62/D80/D84 block below),
  # not the digest-mismatch message this file uses elsewhere for D91-shaped
  # drift. It is no longer active regardless: D100 composed it with D91.
  def test_public_cli_rejects_the_retired_d99_workflow
    _stdout, stderr, status = run_checker(
      roadmap: roadmap_with_tier1_claim(:absent),
      workflow: D99_VCPKG_LIBXML2_CACHE_WORKFLOW_FIXTURE.read
    )

    refute status.success?
    assert_includes stderr, "evidence does not provide the exact 100% line and region gate"
  end

  def test_public_cli_rejects_the_pre_d99_d91_workflow
    _stdout, stderr, status = run_checker(
      roadmap: roadmap_with_tier1_claim(:absent),
      workflow: D91_RELAX_FRONTEND_PERF_MANIFEST_WORKFLOW_FIXTURE.read
    )

    refute status.success?
    assert_includes stderr,
                    "does not match a reviewed active-or-staged performance CI workflow"
  end

  # D100's own composed fixture DOES carry D91's coverage-step release-build
  # line (D91 contributed that half of the composition), so mutating its
  # cache key or timed-loop shape is caught by the digest mismatch, not by
  # `coverage_gate_present?`.
  def test_public_cli_rejects_d100_cache_key_without_the_pinned_llvm_version
    workflow = D100_COMPOSED_WORKFLOW_FIXTURE.read.sub(
      "-llvm-${{ env.LLVM_VERSION }}-",
      "-llvm-unpinned-"
    )

    _stdout, stderr, status = run_checker(
      roadmap: roadmap_with_tier1_claim(:absent),
      workflow: workflow
    )

    refute status.success?
    assert_includes stderr,
                    "does not match a reviewed active-or-staged performance CI workflow"
  end

  def test_public_cli_rejects_d100_cache_key_without_the_hosted_image_version
    workflow = D100_COMPOSED_WORKFLOW_FIXTURE.read.sub(
      "-image-${{ steps.vcpkg_cache_identity.outputs.image_version }}-",
      "-image-unpinned-"
    )

    _stdout, stderr, status = run_checker(
      roadmap: roadmap_with_tier1_claim(:absent),
      workflow: workflow
    )

    refute status.success?
    assert_includes stderr,
                    "does not match a reviewed active-or-staged performance CI workflow"
  end

  def test_public_cli_rejects_drift_in_the_active_d100_workflow
    workflow = D100_COMPOSED_WORKFLOW_FIXTURE.read.sub(
      "for round in 1 2 3 4 5; do",
      "for round in 1 2 3; do"
    )

    _stdout, stderr, status = run_checker(
      roadmap: roadmap_with_tier1_claim(:absent),
      workflow: workflow
    )

    refute status.success?
    assert_includes stderr, "does not match a reviewed active-or-staged performance CI workflow"
  end

  def test_public_cli_rejects_an_active_workflow_without_both_perf_jobs
    workflow = without_workflow_jobs(
      D100_COMPOSED_WORKFLOW_FIXTURE.read,
      "frontend-perf-measure",
      "frontend-perf-gate"
    )

    _stdout, stderr, status = run_checker(
      roadmap: roadmap_with_tier1_claim(:absent),
      workflow: workflow
    )

    refute status.success?
    assert_includes stderr, "does not match a reviewed active-or-staged performance CI workflow"
  end

  def test_public_cli_rejects_retired_d48_with_unchecked_tier1_claim
    _stdout, stderr, status = run_checker(
      roadmap: roadmap_with_tier1_claim(:unchecked),
      workflow: retired_d48_workflow
    )

    refute status.success?
    assert_includes stderr, "does not match a reviewed active-or-staged performance CI workflow"
  end

  def test_public_cli_rejects_retired_d48_without_a_tier1_claim
    _stdout, stderr, status = run_checker(
      roadmap: roadmap_with_tier1_claim(:absent),
      workflow: retired_d48_workflow
    )

    refute status.success?
    assert_includes stderr, "does not match a reviewed active-or-staged performance CI workflow"
  end

  def test_public_cli_requires_active_digest_without_a_tier1_claim
    workflow =
      D100_COMPOSED_WORKFLOW_FIXTURE.read + "\n# unreviewed drift\n"

    _stdout, stderr, status = run_checker(
      roadmap: roadmap_with_tier1_claim(:absent),
      workflow: workflow
    )

    refute status.success?
    assert_includes stderr, "does not match a reviewed active-or-staged performance CI workflow"
  end

  # This block's expected message is `coverage_gate_present?`'s ("evidence
  # does not provide the exact 100% line and region gate"), not the
  # digest-mismatch one used elsewhere in this file for D91-drift cases:
  # `validate_evidence` checks `coverage_gate_present?` before the whole-file
  # digest, and every fixture below genuinely predates D91's coverage-step
  # release-`pycc_rt`-build line (that line did not exist yet at each of
  # these fixtures' own points in history), so they are now caught by that
  # earlier, more specific check rather than by the digest mismatch. Both
  # checks would reject these fixtures; `refute status.success?` is the
  # actual property under test either way.
  def test_public_cli_rejects_the_retired_d56_workflow
    _stdout, stderr, status = run_checker(
      roadmap: roadmap_with_tier1_claim(:absent),
      workflow: D56_SOURCE_AWARE_WORKFLOW_FIXTURE.read
    )

    refute status.success?
    assert_includes stderr, "evidence does not provide the exact 100% line and region gate"
  end

  def test_public_cli_rejects_the_retired_d51_workflow
    _stdout, stderr, status = run_checker(
      roadmap: roadmap_with_tier1_claim(:absent),
      workflow: RETIRED_D51_PAIRED_WORKFLOW.read
    )

    refute status.success?
    assert_includes stderr, "evidence does not provide the exact 100% line and region gate"
  end

  def test_public_cli_rejects_the_retired_d62_workflow
    _stdout, stderr, status = run_checker(
      roadmap: roadmap_with_tier1_claim(:absent),
      workflow: D62_REPLICATED_PAIRED_WORKFLOW_FIXTURE.read
    )

    refute status.success?
    assert_includes stderr, "evidence does not provide the exact 100% line and region gate"
  end

  def test_public_cli_rejects_the_retired_d80_workflow
    _stdout, stderr, status = run_checker(
      roadmap: roadmap_with_tier1_claim(:absent),
      workflow: D80_CONFORMANCE_ORACLE_WORKFLOW_FIXTURE.read
    )

    refute status.success?
    assert_includes stderr, "evidence does not provide the exact 100% line and region gate"
  end

  def test_public_cli_rejects_the_retired_d84_workflow
    _stdout, stderr, status = run_checker(
      roadmap: roadmap_with_tier1_claim(:absent),
      workflow: D84_THROUGHPUT_FLOOR_WORKFLOW_FIXTURE.read
    )

    refute status.success?
    assert_includes stderr, "evidence does not provide the exact 100% line and region gate"
  end

  def test_public_cli_rejects_unreviewed_d56_workflow_drift
    workflow =
      D56_SOURCE_AWARE_WORKFLOW_FIXTURE.read + "\n# unreviewed drift\n"
    _stdout, stderr, status = run_checker(
      roadmap: roadmap_with_tier1_claim(:absent),
      workflow: workflow
    )

    refute status.success?
    assert_includes stderr, "evidence does not provide the exact 100% line and region gate"
  end

  def test_paired_measurement_resolves_the_exact_pull_request_base
    _stdout, stderr, status, output = run_paired_predecessor_resolution(
      event_name: "pull_request"
    )

    assert status.success?, stderr
    assert_equal "sha=exact-base-sha\n", output
  end

  def test_paired_measurement_resolves_the_exact_push_predecessor
    _stdout, stderr, status, output = run_paired_predecessor_resolution(
      event_name: "push"
    )

    assert status.success?, stderr
    assert_equal "sha=exact-before-sha\n", output
  end

  def test_paired_measurement_rejects_an_unsupported_event
    _stdout, stderr, status, output = run_paired_predecessor_resolution(
      event_name: "workflow_dispatch"
    )

    refute status.success?
    assert_includes stderr, "cannot resolve a performance predecessor"
    assert_empty output
  end

  def test_paired_measurement_rejects_missing_or_zero_predecessors
    [
      ["pull_request", "", "unused"],
      ["push", "unused", "0000000000000000000000000000000000000000"]
    ].each do |event_name, pr_base_sha, push_before_sha|
      _stdout, stderr, status, output = run_paired_predecessor_resolution(
        event_name: event_name,
        pr_base_sha: pr_base_sha,
        push_before_sha: push_before_sha
      )

      refute status.success?
      assert_includes stderr, "cannot resolve the exact performance predecessor SHA"
      assert_empty output
    end
  end

  def test_rejects_paired_measurement_without_an_exact_candidate_checkout
    workflow = paired_perf_workflow do |jobs|
      checkout = jobs.fetch("frontend-perf-measure").fetch("steps").find do |step|
        step["name"] == "Check out candidate"
      end
      checkout.fetch("with")["ref"] = "${{ github.event.pull_request.head.sha }}"
    end

    error = assert_raises(RoadmapEvidenceError) do
      validate_perf_gate_baseline_lifecycle(workflow, "ci.yml")
    end
    assert_includes error.message, "reviewed paired measurement job"
  end

  def test_rejects_paired_measurement_without_any_benchmark_contract_path
    %w[
      benches
      Cargo.toml
      Cargo.lock
      rust-toolchain.toml
      rust-toolchain
      .cargo
    ].each do |path|
      workflow = paired_perf_workflow do |jobs|
        verify = jobs.fetch("frontend-perf-measure").fetch("steps").find do |step|
          step["name"] == "Verify exact benchmark revisions"
        end
        verify["run"] = verify.fetch("run").sub("  #{path}\n", "")
      end

      error = assert_raises(RoadmapEvidenceError) do
        validate_perf_gate_baseline_lifecycle(workflow, "ci.yml")
      end
      assert_includes error.message, "reviewed paired measurement job"
    end
  end

  def test_rejects_paired_measurement_without_local_manifest_and_build_script_binding
    [
      "':(glob)crates/**/Cargo.toml'",
      "':(glob)**/build.rs'"
    ].each do |pathspec|
      workflow = paired_perf_workflow do |jobs|
        verify = jobs.fetch("frontend-perf-measure").fetch("steps").find do |step|
          step["name"] == "Verify exact benchmark revisions"
        end
        verify["run"] = verify.fetch("run").sub(pathspec, "':(glob)missing'")
      end

      error = assert_raises(RoadmapEvidenceError) do
        validate_perf_gate_baseline_lifecycle(workflow, "ci.yml")
      end
      assert_includes error.message, "reviewed paired measurement job"
    end
  end

  def test_rejects_paired_measurement_without_both_revision_checks
    [
      'test "$(git -C previous rev-parse HEAD)" = "$EXPECTED_PREDECESSOR_SHA"',
      'test "$(git -C current rev-parse HEAD)" = "$EXPECTED_CURRENT_SHA"'
    ].each do |revision_check|
      workflow = paired_perf_workflow do |jobs|
        verify = jobs.fetch("frontend-perf-measure").fetch("steps").find do |step|
          step["name"] == "Verify exact benchmark revisions"
        end
        verify["run"] = verify.fetch("run").sub(revision_check, "true")
      end

      error = assert_raises(RoadmapEvidenceError) do
        validate_perf_gate_baseline_lifecycle(workflow, "ci.yml")
      end
      assert_includes error.message, "reviewed paired measurement job"
    end
  end

  def test_rejects_paired_measurement_that_runs_candidate_before_sealing_predecessor
    workflow = paired_perf_workflow do |jobs|
      steps = jobs.fetch("frontend-perf-measure").fetch("steps")
      upload_index = steps.index do |step|
        step["name"] == "Upload sealed predecessor frontend timing"
      end
      upload = steps.delete_at(upload_index)
      candidate_index = steps.index do |step|
        step["name"] == "Benchmark exact candidate"
      end
      steps.insert(candidate_index + 1, upload)
    end

    error = assert_raises(RoadmapEvidenceError) do
      validate_perf_gate_baseline_lifecycle(workflow, "ci.yml")
    end
    assert_includes error.message, "reviewed paired measurement job"
  end

  def test_rejects_paired_measurement_with_shared_target_state
    workflow = paired_perf_workflow do |jobs|
      benchmark = jobs.fetch("frontend-perf-measure").fetch("steps").find do |step|
        step["name"] == "Benchmark exact candidate"
      end
      benchmark["run"] = benchmark.fetch("run").sub(
        'current_target="$RUNNER_TEMP/pycc-paired-perf-current"',
        'current_target="$RUNNER_TEMP/pycc-paired-perf-previous"'
      )
    end

    error = assert_raises(RoadmapEvidenceError) do
      validate_perf_gate_baseline_lifecycle(workflow, "ci.yml")
    end
    assert_includes error.message, "reviewed paired measurement job"
  end

  def test_rejects_mutable_actions_in_the_paired_jobs
    [
      ["frontend-perf-measure", "Check out candidate", "actions/checkout@v6"],
      ["frontend-perf-measure", "Upload sealed predecessor frontend timing", "actions/upload-artifact@v4"],
      ["frontend-perf-measure", "Upload candidate frontend timing", "actions/upload-artifact@v4"],
      ["frontend-perf-gate", "Download sealed predecessor frontend timing", "actions/download-artifact@v4"],
      ["frontend-perf-gate", "Download candidate frontend timing", "actions/download-artifact@v4"]
    ].each do |job_name, step_name, mutable_action|
      workflow = paired_perf_workflow do |jobs|
        step = jobs.fetch(job_name).fetch("steps").find do |candidate|
          candidate["name"] == step_name
        end
        step["uses"] = mutable_action
      end

      error = assert_raises(RoadmapEvidenceError) do
        validate_perf_gate_baseline_lifecycle(workflow, "ci.yml")
      end
      assert_includes error.message, "reviewed"
    end
  end

  def test_rejects_paired_uploads_that_include_a_whole_timing_directory
    [
      ["Upload sealed predecessor frontend timing", "previous_benchmark"],
      ["Upload candidate frontend timing", "current_benchmark"]
    ].each do |step_name, step_id|
      workflow = paired_perf_workflow do |jobs|
        upload = jobs.fetch("frontend-perf-measure").fetch("steps").find do |step|
          step["name"] == step_name
        end
        upload.fetch("with")["path"] =
          "${{ steps.#{step_id}.outputs.timing_dir }}"
      end

      error = assert_raises(RoadmapEvidenceError) do
        validate_perf_gate_baseline_lifecycle(workflow, "ci.yml")
      end
      assert_includes error.message, "reviewed paired measurement job"
    end
  end

  def test_rejects_paired_gate_that_checks_out_a_head_controlled_comparator
    workflow = paired_perf_workflow do |jobs|
      checkout = jobs.fetch("frontend-perf-gate").fetch("steps").find do |step|
        step["name"] == "Check out only the reviewed performance checker"
      end
      checkout.fetch("with").delete("ref")
    end

    error = assert_raises(RoadmapEvidenceError) do
      validate_perf_gate_baseline_lifecycle(workflow, "ci.yml")
    end
    assert_includes error.message, "reviewed paired comparison job"
  end

  def test_paired_gate_requires_distinct_numeric_artifact_identities
    _stdout, stderr, status = run_paired_artifact_identity_requirement(
      previous_artifact_id: "123",
      current_artifact_id: "456"
    )
    assert status.success?, stderr

    [
      ["", "456"],
      ["previous", "456"],
      ["123", ""],
      ["123", "current"],
      ["123", "123"]
    ].each do |previous_artifact_id, current_artifact_id|
      _stdout, stderr, status = run_paired_artifact_identity_requirement(
        previous_artifact_id: previous_artifact_id,
        current_artifact_id: current_artifact_id
      )
      refute status.success?
      assert_match(/invalid artifact identity|distinct artifact identities/, stderr)
    end
  end

  def test_rejects_paired_gate_that_downloads_artifacts_by_name
    [
      [
        "Download sealed predecessor frontend timing",
        "frontend-perf-previous"
      ],
      ["Download candidate frontend timing", "frontend-perf-current"]
    ].each do |step_name, artifact_name|
      workflow = paired_perf_workflow do |jobs|
        download = jobs.fetch("frontend-perf-gate").fetch("steps").find do |step|
          step["name"] == step_name
        end
        download.fetch("with").delete("artifact-ids")
        download.fetch("with")["name"] = artifact_name
      end

      error = assert_raises(RoadmapEvidenceError) do
        validate_perf_gate_baseline_lifecycle(workflow, "ci.yml")
      end
      assert_includes error.message, "reviewed paired comparison job"
    end
  end

  def test_rejects_paired_gate_without_valid_flat_id_bound_downloads
    [
      "Download sealed predecessor frontend timing",
      "Download candidate frontend timing"
    ].each do |step_name|
      [[:missing, nil], [:disabled, false], [:invalid, "flatten"]].each do |mutation, value|
        workflow = paired_perf_workflow do |jobs|
          download = jobs.fetch("frontend-perf-gate").fetch("steps").find do |step|
            step["name"] == step_name
          end
          inputs = download.fetch("with")
          if mutation == :missing
            inputs.delete("merge-multiple")
          else
            inputs["merge-multiple"] = value
          end
        end

        error = assert_raises(RoadmapEvidenceError) do
          validate_perf_gate_baseline_lifecycle(workflow, "ci.yml")
        end
        assert_includes error.message, "reviewed paired comparison job"
      end
    end
  end

  def test_paired_timing_requirement_accepts_exactly_two_regular_files
    _stdout, stderr, status = run_paired_timing_requirement do |root|
      %w[previous current].each do |revision|
        FileUtils.mkdir_p(root / revision)
        (root / revision / "estimates.json").write("{}")
      end
    end

    assert status.success?, stderr
  end

  def test_paired_timing_requirement_rejects_either_missing_estimate
    %w[previous current].each do |present_revision|
      _stdout, stderr, status = run_paired_timing_requirement do |root|
        FileUtils.mkdir_p(root / present_revision)
        (root / present_revision / "estimates.json").write("{}")
      end

      refute status.success?
      missing_revision =
        present_revision == "previous" ? "current" : "previous"
      assert_includes stderr, "missing #{missing_revision}/estimates.json"
    end
  end

  def test_paired_timing_requirement_rejects_an_extra_file
    _stdout, stderr, status = run_paired_timing_requirement do |root|
      %w[previous current].each do |revision|
        FileUtils.mkdir_p(root / revision)
        (root / revision / "estimates.json").write("{}")
      end
      (root / "unexpected.txt").write("extra")
    end

    refute status.success?
    assert_includes stderr, "exactly two regular files"
  end

  def test_paired_timing_requirement_rejects_a_symlink
    _stdout, stderr, status = run_paired_timing_requirement do |root|
      FileUtils.mkdir_p(root / "previous")
      FileUtils.mkdir_p(root / "current")
      (root / "previous" / "estimates.json").write("{}")
      File.symlink(
        root / "previous" / "estimates.json",
        root / "current" / "estimates.json"
      )
    end

    refute status.success?
    assert_includes stderr, "missing current/estimates.json"
  end

  def test_replicated_timing_requirement_accepts_exactly_ten_regular_files
    _stdout, stderr, status = run_replicated_timing_requirement do |root|
      %w[previous current].each do |revision|
        FileUtils.mkdir_p(root / revision)
        5.times do |index|
          (root / revision / "round-#{index + 1}.json").write("{}")
        end
      end
    end

    assert status.success?, stderr
  end

  def test_replicated_timing_requirement_rejects_a_missing_fixed_sample
    %w[previous current].each do |missing_revision|
      _stdout, stderr, status = run_replicated_timing_requirement do |root|
        %w[previous current].each do |revision|
          FileUtils.mkdir_p(root / revision)
          5.times do |index|
            next if revision == missing_revision && index == 2

            (root / revision / "round-#{index + 1}.json").write("{}")
          end
        end
      end

      refute status.success?
      assert_includes stderr, "missing #{missing_revision}/round-3.json"
    end
  end

  def test_replicated_timing_requirement_rejects_an_extra_file
    _stdout, stderr, status = run_replicated_timing_requirement do |root|
      %w[previous current].each do |revision|
        FileUtils.mkdir_p(root / revision)
        5.times do |index|
          (root / revision / "round-#{index + 1}.json").write("{}")
        end
      end
      (root / "unexpected.txt").write("extra")
    end

    refute status.success?
    assert_includes stderr, "exactly two directories and ten regular files"
  end

  def test_replicated_timing_requirement_rejects_an_extra_empty_directory
    _stdout, stderr, status = run_replicated_timing_requirement do |root|
      %w[previous current].each do |revision|
        FileUtils.mkdir_p(root / revision)
        5.times do |index|
          (root / revision / "round-#{index + 1}.json").write("{}")
        end
      end
      FileUtils.mkdir_p(root / "unexpected")
    end

    refute status.success?
    assert_includes stderr, "exactly two directories and ten regular files"
  end

  def test_replicated_timing_requirement_rejects_a_symlink
    _stdout, stderr, status = run_replicated_timing_requirement do |root|
      %w[previous current].each do |revision|
        FileUtils.mkdir_p(root / revision)
        5.times do |index|
          (root / revision / "round-#{index + 1}.json").write("{}")
        end
      end
      File.unlink(root / "current/round-5.json")
      File.symlink(root / "current/round-4.json", root / "current/round-5.json")
    end

    refute status.success?
    assert_includes stderr, "missing current/round-5.json"
  end

  def test_rejects_replicated_measurement_with_a_changed_fixed_sample_count
    ["Benchmark exact predecessor", "Benchmark exact candidate"].each do |step_name|
      workflow = replicated_perf_workflow do |jobs|
        benchmark = jobs.fetch("frontend-perf-measure").fetch("steps").find do |step|
          step["name"] == step_name
        end
        benchmark["run"] = benchmark.fetch("run").sub(
          "for round in 1 2 3 4 5; do",
          "for round in 1 2 3; do"
        )
      end

      error = assert_raises(RoadmapEvidenceError) do
        validate_source_aware_perf_gate_lifecycle(workflow, "ci.yml")
      end
      assert_includes error.message, "reviewed source-aware measurement job"
    end
  end

  def test_rejects_replicated_gate_with_a_paired_single_sample_comparator
    workflow = replicated_perf_workflow do |jobs|
      jobs["frontend-perf-gate"] =
        Marshal.load(Marshal.dump(PAIRED_PERF_GATE_JOB))
    end

    error = assert_raises(RoadmapEvidenceError) do
      validate_source_aware_perf_gate_lifecycle(workflow, "ci.yml")
    end
    assert_includes error.message, "reviewed source-aware comparison job"
  end

  def test_rejects_replicated_measurement_that_runs_candidate_before_sealing_predecessor
    workflow = replicated_perf_workflow do |jobs|
      steps = jobs.fetch("frontend-perf-measure").fetch("steps")
      upload_index = steps.index do |step|
        step["name"] == "Upload sealed predecessor frontend timing"
      end
      upload = steps.delete_at(upload_index)
      candidate_index = steps.index do |step|
        step["name"] == "Benchmark exact candidate"
      end
      steps.insert(candidate_index + 1, upload)
    end

    error = assert_raises(RoadmapEvidenceError) do
      validate_source_aware_perf_gate_lifecycle(workflow, "ci.yml")
    end
    assert_includes error.message, "reviewed source-aware measurement job"
  end

  def test_rejects_mutable_actions_in_the_replicated_jobs
    [
      ["frontend-perf-measure", "Upload candidate frontend timing", "actions/upload-artifact@v4"],
      ["frontend-perf-gate", "Download candidate frontend timing", "actions/download-artifact@v4"]
    ].each do |job_name, step_name, mutable_action|
      workflow = replicated_perf_workflow do |jobs|
        step = jobs.fetch(job_name).fetch("steps").find do |candidate|
          candidate["name"] == step_name
        end
        step["uses"] = mutable_action
      end

      error = assert_raises(RoadmapEvidenceError) do
        validate_source_aware_perf_gate_lifecycle(workflow, "ci.yml")
      end
      assert_includes error.message, "reviewed"
    end
  end

  def test_rejects_replicated_gate_without_flat_id_bound_downloads
    workflow = replicated_perf_workflow do |jobs|
      download = jobs.fetch("frontend-perf-gate").fetch("steps").find do |step|
        step["name"] == "Download candidate frontend timing"
      end
      download.fetch("with").delete("merge-multiple")
    end

    error = assert_raises(RoadmapEvidenceError) do
      validate_source_aware_perf_gate_lifecycle(workflow, "ci.yml")
    end
    assert_includes error.message, "reviewed source-aware comparison job"
  end

  def test_rejects_replicated_gate_with_a_head_controlled_comparator
    workflow = replicated_perf_workflow do |jobs|
      checkout = jobs.fetch("frontend-perf-gate").fetch("steps").find do |step|
        step["name"] == "Check out only the reviewed performance checker"
      end
      checkout.fetch("with").delete("ref")
    end

    error = assert_raises(RoadmapEvidenceError) do
      validate_source_aware_perf_gate_lifecycle(workflow, "ci.yml")
    end
    assert_includes error.message, "reviewed source-aware comparison job"
  end

  def test_rejects_replicated_gate_without_executable_input_binding
    workflow = replicated_perf_workflow do |jobs|
      compare = jobs.fetch("frontend-perf-gate").fetch("steps").find do |step|
        step["name"] == "Compare exact predecessor and candidate"
      end
      compare.delete("env")
    end

    error = assert_raises(RoadmapEvidenceError) do
      validate_source_aware_perf_gate_lifecycle(workflow, "ci.yml")
    end
    assert_includes error.message, "reviewed source-aware comparison job"
  end

  def test_rejects_paired_gate_that_can_skip_the_comparison
    workflow = paired_perf_workflow do |jobs|
      compare = jobs.fetch("frontend-perf-gate").fetch("steps").find do |step|
        step["name"] == "Compare exact predecessor and candidate"
      end
      compare["if"] = "${{ false }}"
    end

    error = assert_raises(RoadmapEvidenceError) do
      validate_perf_gate_baseline_lifecycle(workflow, "ci.yml")
    end
    assert_includes error.message, "reviewed paired comparison job"
  end

  def test_rejects_a_retired_d48_like_comparison_gate
    workflow = paired_perf_workflow do |jobs|
      gate = jobs.fetch("frontend-perf-gate")
      compare = gate.fetch("steps").find do |step|
        step["name"] == "Compare exact predecessor and candidate"
      end
      compare["name"] = "Compare against canonical main baseline"
      compare["run"] =
        "ruby scripts/check_perf_regression.rb current.json previous.json"
    end

    error = assert_raises(RoadmapEvidenceError) do
      validate_perf_gate_baseline_lifecycle(workflow, "ci.yml")
    end
    assert_includes error.message, "reviewed paired comparison job"
  end

  def test_rejects_paired_gate_without_a_measurement_dependency
    workflow = paired_perf_workflow do |jobs|
      jobs.fetch("frontend-perf-gate").delete("needs")
    end

    error = assert_raises(RoadmapEvidenceError) do
      validate_perf_gate_baseline_lifecycle(workflow, "ci.yml")
    end
    assert_includes error.message, "reviewed paired comparison job"
  end

  def test_rejects_a_paired_gate_without_a_measurement_job
    workflow = paired_perf_workflow do |jobs|
      jobs.delete("frontend-perf-measure")
    end

    error = assert_raises(RoadmapEvidenceError) do
      validate_perf_gate_baseline_lifecycle(workflow, "ci.yml")
    end
    assert_includes error.message, "requires frontend-perf-measure"
  end

  def test_rejects_paired_perf_jobs_not_required_by_ci_gate
    workflow = paired_perf_workflow do |jobs|
      jobs.fetch("ci-gate").fetch("needs").delete("frontend-perf-measure")
    end

    error = assert_raises(RoadmapEvidenceError) do
      validate_perf_gate_baseline_lifecycle(workflow, "ci.yml")
    end
    assert_includes error.message, "reviewed fail-closed aggregate job"
  end

  def test_rejects_a_noop_paired_perf_ci_gate
    workflow = paired_perf_workflow do |jobs|
      jobs.fetch("ci-gate")["steps"] = [{ "run" => "true" }]
    end

    error = assert_raises(RoadmapEvidenceError) do
      validate_perf_gate_baseline_lifecycle(workflow, "ci.yml")
    end
    assert_includes error.message, "reviewed fail-closed aggregate job"
  end

  def test_rejects_a_paired_perf_ci_gate_that_can_be_skipped
    workflow = paired_perf_workflow do |jobs|
      jobs.fetch("ci-gate").delete("if")
    end

    error = assert_raises(RoadmapEvidenceError) do
      validate_perf_gate_baseline_lifecycle(workflow, "ci.yml")
    end
    assert_includes error.message, "reviewed fail-closed aggregate job"
  end

  def test_rejects_each_missing_paired_perf_ci_gate_result_check
    PAIRED_PERF_CI_GATE_NEEDS.each do |job_name|
      workflow = paired_perf_workflow do |jobs|
        gate_step = jobs.fetch("ci-gate").fetch("steps").first
        predicate = "needs.#{job_name}.result != 'success'"
        gate_step["if"] = gate_step.fetch("if").sub(
          /(?: \|\| )?#{Regexp.escape(predicate)}/,
          ""
        )
      end

      error = assert_raises(RoadmapEvidenceError, job_name) do
        validate_perf_gate_baseline_lifecycle(workflow, "ci.yml")
      end
      assert_includes(
        error.message,
        "reviewed fail-closed aggregate job",
        job_name
      )
    end
  end

  def test_rejects_changed_tier1_matrix_workflow
    repository_root = REPOSITORY_ROOT
    workflow = (repository_root / ".github/workflows/ci.yml").read.sub(
      "macos-15-intel",
      "macos-14"
    )
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      ## Current delivery status

      ### v0.1 acceptance checklist

      - [x] The five-target native CI matrix and one cross-host compilation path are live on `main`. <!-- roadmap-evidence: ci-tier1-cross-compile -->
    MARKDOWN

    _stdout, stderr, status = run_checker(roadmap: roadmap, workflow: workflow)

    refute status.success?
    assert_includes stderr, "does not match a reviewed active-or-staged performance CI workflow"
  end

  def test_requires_the_hard_coverage_gate_while_its_roadmap_claim_is_unchecked
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      ## Current delivery status

      ### v0.1 acceptance checklist

      - [ ] The 100% line and region coverage gate is required and green for the current slice.
    MARKDOWN

    _stdout, stderr, status = run_checker(
      roadmap: roadmap,
      workflow: coverage_workflow("true")
    )

    refute status.success?
    assert_includes stderr, "does not provide the exact 100% line and region gate"
  end

  def test_rejects_coverage_evidence_when_the_threshold_is_lowered
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      ## Current delivery status

      ### v0.1 acceptance checklist

      - [x] The 100% line and region coverage gate is required and green for the current slice. <!-- roadmap-evidence: ci-build-test-coverage-100 -->
    MARKDOWN

    _stdout, stderr, status = run_checker(
      roadmap: roadmap,
      workflow: coverage_workflow(
        "run_isolated \"$TRUSTED_COV\" llvm-cov --workspace " \
        "--fail-under-lines 99 --fail-under-regions 100"
      )
    )

    refute status.success?
    assert_includes stderr, "does not provide the exact 100% line and region gate"
  end

  def test_rejects_a_coverage_step_that_can_be_skipped
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      ## Current delivery status

      ### v0.1 acceptance checklist

      - [x] The 100% line and region coverage gate is required and green for the current slice. <!-- roadmap-evidence: ci-build-test-coverage-100 -->
    MARKDOWN
    workflow = coverage_workflow.sub(
      "#{COVERAGE_STEP_HEADER}\n        run:",
      "#{COVERAGE_STEP_HEADER}\n        if: false\n        run:"
    )

    _stdout, stderr, status = run_checker(roadmap: roadmap, workflow: workflow)

    refute status.success?
    assert_includes stderr, "coverage evidence must run unconditionally"
  end

  def test_rejects_a_preceding_step_that_can_shadow_cargo
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      ## Current delivery status

      ### v0.1 acceptance checklist

      - [x] The 100% line and region coverage gate is required and green for the current slice. <!-- roadmap-evidence: ci-build-test-coverage-100 -->
    MARKDOWN
    workflow = coverage_workflow.sub(
      "    steps:\n",
      "    steps:\n      - name: Shadow cargo\n        run: echo fake-cargo >> \"$GITHUB_PATH\"\n"
    )

    _stdout, stderr, status = run_checker(roadmap: roadmap, workflow: workflow)

    refute status.success?
    assert_includes stderr, "coverage setup steps do not match the trusted sequence"
  end

  def test_rejects_environment_that_can_shadow_coverage_tools
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      ## Current delivery status

      ### v0.1 acceptance checklist

      - [x] The 100% line and region coverage gate is required and green for the current slice. <!-- roadmap-evidence: ci-build-test-coverage-100 -->
    MARKDOWN
    workflow = coverage_workflow.sub(
      "  CARGO_LLVM_COV_VERSION: \"0.8.7\"\n",
      "  CARGO_LLVM_COV_VERSION: \"0.8.7\"\n  PATH: /tmp/fake-bin\n"
    )

    _stdout, stderr, status = run_checker(roadmap: roadmap, workflow: workflow)

    refute status.success?
    assert_includes stderr, "coverage workflow environment does not match the trusted values"
  end

  def test_accepts_explicit_continue_on_error_false
    workflow = coverage_workflow.sub(
      "#{COVERAGE_STEP_HEADER}\n        run:",
      "#{COVERAGE_STEP_HEADER}\n        continue-on-error: false\n        run:"
    )

    assert coverage_gate_present?(workflow, "ci.yml")
  end

  def test_rejects_a_coverage_job_with_dependencies
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      ## Current delivery status

      ### v0.1 acceptance checklist

      - [x] The 100% line and region coverage gate is required and green for the current slice. <!-- roadmap-evidence: ci-build-test-coverage-100 -->
    MARKDOWN
    workflow = coverage_workflow
               .sub(
                 "jobs:\n",
                 "jobs:\n  setup:\n    if: false\n    steps: []\n"
               )
               .sub(
                 "  build-test-coverage:\n",
                 "  build-test-coverage:\n    needs: setup\n"
               )

    _stdout, stderr, status = run_checker(roadmap: roadmap, workflow: workflow)

    refute status.success?
    assert_includes stderr, "coverage evidence must not depend on other jobs"
  end

  def test_rejects_continue_on_error_for_the_coverage_job
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      ## Current delivery status

      ### v0.1 acceptance checklist

      - [x] The 100% line and region coverage gate is required and green for the current slice. <!-- roadmap-evidence: ci-build-test-coverage-100 -->
    MARKDOWN
    workflow = coverage_workflow.sub(
      "  build-test-coverage:\n",
      "  build-test-coverage:\n    continue-on-error: true\n"
    )

    _stdout, stderr, status = run_checker(roadmap: roadmap, workflow: workflow)

    refute status.success?
    assert_includes stderr, "coverage job must propagate failures"
  end

  def test_rejects_a_custom_shell_for_the_coverage_step
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      ## Current delivery status

      ### v0.1 acceptance checklist

      - [x] The 100% line and region coverage gate is required and green for the current slice. <!-- roadmap-evidence: ci-build-test-coverage-100 -->
    MARKDOWN
    workflow = coverage_workflow.sub(
      "#{COVERAGE_STEP_HEADER}\n        run:",
      "#{COVERAGE_STEP_HEADER}\n        shell: 'true {0}'\n        run:"
    )

    _stdout, stderr, status = run_checker(roadmap: roadmap, workflow: workflow)

    refute status.success?
    assert_includes stderr, "coverage step must use the default shell"
  end

  def test_rejects_workflow_and_job_run_defaults
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      ## Current delivery status

      ### v0.1 acceptance checklist

      - [x] The 100% line and region coverage gate is required and green for the current slice. <!-- roadmap-evidence: ci-build-test-coverage-100 -->
    MARKDOWN
    inherited_defaults = [
      coverage_workflow.sub(
        "jobs:\n",
        "defaults:\n  run:\n    shell: 'true {0}'\njobs:\n"
      ),
      coverage_workflow.sub(
        "  build-test-coverage:\n",
        "  build-test-coverage:\n    defaults:\n      run:\n        shell: 'true {0}'\n"
      )
    ]

    inherited_defaults.each do |workflow|
      _stdout, stderr, status = run_checker(roadmap: roadmap, workflow: workflow)

      refute status.success?
      assert_includes stderr, "coverage evidence must not inherit run defaults"
    end
  end

  def test_rejects_coverage_evidence_not_scheduled_for_pull_requests
    roadmap = <<~MARKDOWN
      # pycc Roadmap

      ## Current delivery status

      ### v0.1 acceptance checklist

      - [x] The 100% line and region coverage gate is required and green for the current slice. <!-- roadmap-evidence: ci-build-test-coverage-100 -->
    MARKDOWN
    workflow = coverage_workflow.sub("pull_request:", "workflow_dispatch:")

    _stdout, stderr, status = run_checker(roadmap: roadmap, workflow: workflow)

    refute status.success?
    assert_includes stderr, "coverage evidence must run on every pull request"
  end

  def test_repository_ci_runs_the_self_tests_and_checker
    repository_root = REPOSITORY_ROOT
    workflow = Psych.load((repository_root / ".github/workflows/ci.yml").read)
    run_blocks = workflow
                 .fetch("jobs")
                 .fetch("build-test-coverage")
                 .fetch("steps")
                 .map { |step| step["run"] }
                 .compact
    commands = run_blocks.flat_map { |run| run.lines.map(&:strip) }

    assert_includes commands, "ruby scripts/test_check_roadmap_evidence.rb"
    assert_includes commands, "ruby scripts/check_roadmap_evidence.rb"
    assert_includes commands,
                    "printf 'LLVM_SYS_221_PREFIX=%s\\n' " \
                    "\"$LLVM_SYS_221_PREFIX_VALUE\" >> \"$GITHUB_ENV\""
    assert_includes commands, 'sudo -u nobody env -i "${ISOLATED_ENV[@]}" "$@"'
    assert_includes commands, 'sudo chmod -R o+rX "$TRUSTED_TOOLCHAIN"'
    assert_includes commands,
                    'ln -s "$ISOLATED_ROOT/target" "$GITHUB_WORKSPACE/target"'
    assert_includes commands,
                    'run_isolated "$TRUSTED_CARGO" build ' \
                    "--target x86_64-apple-darwin -p pycc_rt"
    assert_operator commands.index("cargo build --workspace"),
                    :<,
                    commands.index("cargo test --workspace -- --include-ignored")
  end

  # Issue #229 (Phase 3 activation): tests for validate_pages_performance_lifecycle.
  # These tests use synthetic ci.yml fixtures (not the live ci.yml) to test
  # the lifecycle validator's structural invariants in isolation.  The live
  # ci.yml now has a pages-accessibility job (activated in Merge 2 of #199),
  # and test_d199_pages_accessibility_workflow_is_active_and_reviewed above
  # verifies the live ci.yml passes the lifecycle validator.

  # A minimal valid pages-performance job for lifecycle testing.  The
  # lifecycle validator checks structural invariants (existence, ci-gate
  # wiring, permissions, no continue-on-error, not push-only) -- not the
  # exact step content, which is reviewed by
  # check_pages_performance_budget.rb's own test suite.
  def pages_perf_job_yaml(overrides = {})
    job = {
      "runs-on" => "ubuntu-latest",
      "permissions" => { "contents" => "read" },
      "steps" => [
        { "uses" => "actions/checkout@d23441a48e516b6c34aea4fa41551a30e30af803",
          "with" => { "persist-credentials" => false } },
        { "name" => "Run hermetic Pages performance budget gate",
          "run" => "ruby scripts/check_pages_performance_budget.rb\n" }
      ]
    }
    job.merge!(overrides)
    { "pages-performance" => job }.to_yaml
  end

  # A minimal ci-gate that requires pages-performance and checks its result.
  def ci_gate_with_pages_perf_yaml(overrides = {})
    gate = {
      "needs" => ["build-test-coverage", "pages-performance"],
      "if" => "always()",
      "runs-on" => "ubuntu-latest",
      "permissions" => {},
      "steps" => [
        { "name" => "Fail unless every required job succeeded",
          "if" => "needs.build-test-coverage.result != 'success' || needs.pages-performance.result != 'success'",
          "run" => "echo fail\nexit 1\n" }
      ]
    }
    gate.merge!(overrides)
    { "ci-gate" => gate }.to_yaml
  end

  # Build a complete workflow YAML with the given job and ci-gate YAML strings.
  def pages_perf_workflow_yaml(pages_job: nil, ci_gate: nil)
    jobs = {
      "build-test-coverage" => {
        "runs-on" => "macos-14",
        "steps" => [{ "name" => "test", "run" => "echo hi\n" }]
      }
    }
    jobs.merge!(Psych.load(pages_job)) if pages_job
    jobs.merge!(Psych.load(ci_gate)) if ci_gate
    {
      "on" => { "push" => { "branches" => ["main"] }, "pull_request" => nil },
      "permissions" => { "contents" => "read" },
      "jobs" => jobs
    }.to_yaml
  end

  def test_pages_performance_lifecycle_accepts_a_valid_workflow
    workflow = pages_perf_workflow_yaml(
      pages_job: pages_perf_job_yaml,
      ci_gate: ci_gate_with_pages_perf_yaml
    )
    assert validate_pages_performance_lifecycle(workflow, "ci.yml")
  end

  def test_pages_performance_lifecycle_rejects_a_missing_job
    workflow = pages_perf_workflow_yaml(
      pages_job: nil,
      ci_gate: ci_gate_with_pages_perf_yaml
    )
    error = assert_raises(RoadmapEvidenceError) do
      validate_pages_performance_lifecycle(workflow, "ci.yml")
    end
    assert_includes error.message, "pages-performance job is required"
  end

  def test_pages_performance_lifecycle_rejects_a_job_missing_from_ci_gate_needs
    ci_gate = ci_gate_with_pages_perf_yaml(
      "needs" => ["build-test-coverage"]
    )
    workflow = pages_perf_workflow_yaml(
      pages_job: pages_perf_job_yaml,
      ci_gate: ci_gate
    )
    error = assert_raises(RoadmapEvidenceError) do
      validate_pages_performance_lifecycle(workflow, "ci.yml")
    end
    assert_includes error.message, "ci-gate must require pages-performance"
  end

  def test_pages_performance_lifecycle_rejects_continue_on_error
    pages_job = pages_perf_job_yaml("continue-on-error" => true)
    workflow = pages_perf_workflow_yaml(
      pages_job: pages_job,
      ci_gate: ci_gate_with_pages_perf_yaml
    )
    error = assert_raises(RoadmapEvidenceError) do
      validate_pages_performance_lifecycle(workflow, "ci.yml")
    end
    assert_includes error.message, "pages-performance must propagate failures"
  end

  def test_pages_performance_lifecycle_rejects_a_push_only_job
    pages_job = pages_perf_job_yaml("if" => "github.event_name == 'push'")
    workflow = pages_perf_workflow_yaml(
      pages_job: pages_job,
      ci_gate: ci_gate_with_pages_perf_yaml
    )
    error = assert_raises(RoadmapEvidenceError) do
      validate_pages_performance_lifecycle(workflow, "ci.yml")
    end
    assert_includes error.message, "pages-performance must not be push-only"
  end

  def test_pages_performance_lifecycle_rejects_wrong_permissions
    pages_job = pages_perf_job_yaml(
      "permissions" => { "contents" => "write" }
    )
    workflow = pages_perf_workflow_yaml(
      pages_job: pages_job,
      ci_gate: ci_gate_with_pages_perf_yaml
    )
    error = assert_raises(RoadmapEvidenceError) do
      validate_pages_performance_lifecycle(workflow, "ci.yml")
    end
    assert_includes error.message, "pages-performance must have contents: read permission"
  end

  def test_pages_performance_lifecycle_rejects_missing_permissions
    pages_job_hash = Psych.load(pages_perf_job_yaml)
    pages_job_hash["pages-performance"].delete("permissions")
    workflow = pages_perf_workflow_yaml(
      pages_job: pages_job_hash.to_yaml,
      ci_gate: ci_gate_with_pages_perf_yaml
    )
    error = assert_raises(RoadmapEvidenceError) do
      validate_pages_performance_lifecycle(workflow, "ci.yml")
    end
    assert_includes error.message, "pages-performance must declare explicit permissions"
  end

  def test_pages_performance_lifecycle_rejects_missing_fail_step_check
    ci_gate = ci_gate_with_pages_perf_yaml(
      "steps" => [
        { "name" => "Fail unless every required job succeeded",
          "if" => "needs.build-test-coverage.result != 'success'",
          "run" => "echo fail\nexit 1\n" }
      ]
    )
    workflow = pages_perf_workflow_yaml(
      pages_job: pages_perf_job_yaml,
      ci_gate: ci_gate
    )
    error = assert_raises(RoadmapEvidenceError) do
      validate_pages_performance_lifecycle(workflow, "ci.yml")
    end
    assert_includes error.message, "ci-gate fail step must check needs.pages-performance.result"
  end

  def test_pages_performance_lifecycle_rejects_extra_permissions
    pages_job = pages_perf_job_yaml(
      "permissions" => { "contents" => "read", "actions" => "read" }
    )
    workflow = pages_perf_workflow_yaml(
      pages_job: pages_job,
      ci_gate: ci_gate_with_pages_perf_yaml
    )
    error = assert_raises(RoadmapEvidenceError) do
      validate_pages_performance_lifecycle(workflow, "ci.yml")
    end
    assert_includes error.message, "pages-performance must have only contents: read permission"
  end

  # Issue #229 (Phase 3 activation): the D229 pages-performance ci.yml
  # digest is now the live ci.yml.  This test verifies the digest
  # constant is in the accepted array.
  def test_d229_pages_performance_digest_is_staged
    assert_includes REVIEWED_PERF_CI_WORKFLOW_SHA256S,
                    D229_PAGES_PERFORMANCE_CI_WORKFLOW_SHA256
  end

  # Issue #229 (Phase 3 activation): validate_pages_performance_lifecycle
  # gates the pages-performance validation on the live ci.yml digest.
  # Now that the live ci.yml matches D229_PAGES_PERFORMANCE_CI_WORKFLOW_SHA256,
  # the validator ENFORCES the pages-performance lifecycle validation.
  # For pre-D229 accepted digests (D100/D112/D114), the validator SKIPS
  # the pages-performance lifecycle validation (the job does not exist in
  # those shapes).  For an unknown digest (not in the accepted array), the
  # validator enforces fail-closed -- validate_evidence's own digest check
  # rejects unknown digests before this function runs in production, so
  # that default only matters for direct unit tests with synthetic workflows.

  def test_pages_performance_lifecycle_skips_validation_for_a_pre_d229_digest
    # The live D114 ci.yml fixture has a pre-D229 digest (no
    # pages-performance job).  The validator must skip and return true
    # rather than rejecting the missing job.
    workflow_text = D114_FRONTEND_PERF_THRESHOLD_WORKFLOW_FIXTURE.read
    digest = Digest::SHA256.hexdigest(workflow_text)
    assert_includes REVIEWED_PERF_CI_WORKFLOW_SHA256S, digest
    refute_equal D229_PAGES_PERFORMANCE_CI_WORKFLOW_SHA256, digest
    assert validate_pages_performance_lifecycle(workflow_text, "ci.yml")
  end

  def test_pages_performance_lifecycle_skips_validation_for_d100_digest
    # The D100 fixture is another accepted pre-D229 digest.
    workflow_text = D100_COMPOSED_WORKFLOW_FIXTURE.read
    digest = Digest::SHA256.hexdigest(workflow_text)
    assert_includes REVIEWED_PERF_CI_WORKFLOW_SHA256S, digest
    refute_equal D229_PAGES_PERFORMANCE_CI_WORKFLOW_SHA256, digest
    assert validate_pages_performance_lifecycle(workflow_text, "ci.yml")
  end

  def test_pages_performance_lifecycle_enforces_for_an_unknown_digest
    # A synthetic workflow with no pages-performance job and a digest that
    # matches no reviewed workflow.  The validator must enforce fail-closed
    # (reject the missing job) rather than skipping.
    workflow = pages_perf_workflow_yaml(
      pages_job: nil,
      ci_gate: ci_gate_with_pages_perf_yaml
    )
    digest = Digest::SHA256.hexdigest(workflow)
    refute_includes REVIEWED_PERF_CI_WORKFLOW_SHA256S, digest
    error = assert_raises(RoadmapEvidenceError) do
      validate_pages_performance_lifecycle(workflow, "ci.yml")
    end
    assert_includes error.message, "pages-performance job is required"
  end

  def test_pages_performance_lifecycle_enforces_for_an_unknown_digest_with_valid_job
    # A synthetic workflow WITH a valid pages-performance job but a digest
    # that matches no reviewed workflow.  The validator must still enforce
    # (and pass) the structural validation -- the digest gate only skips
    # for known pre-D229 digests, not for all unknown digests.
    workflow = pages_perf_workflow_yaml(
      pages_job: pages_perf_job_yaml,
      ci_gate: ci_gate_with_pages_perf_yaml
    )
    digest = Digest::SHA256.hexdigest(workflow)
    refute_includes REVIEWED_PERF_CI_WORKFLOW_SHA256S, digest
    assert validate_pages_performance_lifecycle(workflow, "ci.yml")
  end

  # Issue #199 (D-103 stage): the D199 pages-accessibility ci.yml
  # digest is staged but not yet active.  These tests verify the
  # staged fixture exists, its digest is in the accepted array, and
  # the D199 ci-gate shape is accepted by the lifecycle validator.

  def test_d199_pages_accessibility_digest_is_staged
    assert_includes REVIEWED_PERF_CI_WORKFLOW_SHA256S,
                    D199_PAGES_ACCESSIBILITY_CI_WORKFLOW_SHA256
  end

  def test_d199_pages_accessibility_workflow_fixture_digest_matches_constant
    assert_equal(
      D199_PAGES_ACCESSIBILITY_CI_WORKFLOW_SHA256,
      Digest::SHA256.file(D199_PAGES_ACCESSIBILITY_WORKFLOW_FIXTURE).hexdigest
    )
  end

  def test_d211_coverage_badge_binding_workflow_fixture_digest_matches_constant
    assert_equal(
      D211_COVERAGE_BADGE_BINDING_CI_WORKFLOW_SHA256,
      Digest::SHA256.file(D211_COVERAGE_BADGE_BINDING_WORKFLOW_FIXTURE).hexdigest
    )
  end

  def test_live_ci_yml_is_an_exact_python_3147_transition_shape
    live_digest = Digest::SHA256.file(
      REPOSITORY_ROOT / ".github/workflows/ci.yml"
    ).hexdigest
    assert_includes(
      [
        D211_COVERAGE_BADGE_BINDING_CI_WORKFLOW_SHA256,
        PY3147_ORACLE_CI_WORKFLOW_SHA256
      ],
      live_digest
    )
  end

  def test_python_3147_oracle_workflow_fixture_is_reviewed
    staged_digest = Digest::SHA256.file(PY3147_ORACLE_WORKFLOW_FIXTURE).hexdigest

    assert_equal PY3147_ORACLE_CI_WORKFLOW_SHA256, staged_digest
    assert_includes REVIEWED_PERF_CI_WORKFLOW_SHA256S, staged_digest
  end

  def test_d199_pages_accessibility_workflow_is_not_the_live_ci_yml
    # The D199 fixture was the live ci.yml before the D211 activation.
    # After issue #211's activation merge, the live ci.yml is the D211 shape.
    live_digest = Digest::SHA256.file(
      REPOSITORY_ROOT / ".github/workflows/ci.yml"
    ).hexdigest
    refute_equal D199_PAGES_ACCESSIBILITY_CI_WORKFLOW_SHA256, live_digest
  end

  def test_d199_pages_accessibility_ci_gate_has_pages_accessibility_in_needs
    assert_includes D199_PAGES_ACCESSIBILITY_CI_GATE_JOB.fetch("needs"),
                    "pages-accessibility"
  end

  def test_d199_pages_accessibility_ci_gate_has_pages_accessibility_in_fail_condition
    fail_step =
      D199_PAGES_ACCESSIBILITY_CI_GATE_JOB.fetch("steps").find do |step|
        step["name"] == "Fail unless every required job succeeded"
      end
    assert_includes fail_step.fetch("if"),
                    "needs.pages-accessibility.result != 'success'"
  end

  def test_d199_pages_accessibility_ci_gate_has_eight_needs_entries
    assert_equal 8, D199_PAGES_ACCESSIBILITY_CI_GATE_JOB.fetch("needs").length
  end

  def test_accepted_perf_ci_gate_jobs_includes_d199_shape
    assert_includes ACCEPTED_PERF_CI_GATE_JOBS,
                    D199_PAGES_ACCESSIBILITY_CI_GATE_JOB
  end

  def test_d171_workflow_is_digest_bound_reviewed_and_structurally_valid
    digest = Digest::SHA256.file(D171_WORKFLOW_FIXTURE).hexdigest

    assert_equal D171_CHANGE_AWARE_CI_WORKFLOW_SHA256, digest
    assert_includes REVIEWED_PERF_CI_WORKFLOW_SHA256S, digest
    assert validate_d171_ci_routing(D171_WORKFLOW_FIXTURE.read, D171_WORKFLOW_FIXTURE.to_s)

    stdout, stderr, status = run_checker(
      roadmap: "# pycc Roadmap\n",
      workflow: D171_WORKFLOW_FIXTURE.read
    )
    assert status.success?, stderr
    assert_includes stdout, "Roadmap evidence policy passed."
  end

  def test_d171_checker_keeps_the_current_live_workflow_compatible
    stdout, stderr, status = run_checker(
      roadmap: "# pycc Roadmap\n",
      workflow: ACTIVE_D100_COMPOSE_D91_D99_WORKFLOW.read
    )

    assert status.success?, stderr
    assert_includes stdout, "Roadmap evidence policy passed."
  end

  def test_d171_rejects_each_mutable_or_credential_persisting_checkout
    locations = d171_checkout_locations(d171_workflow)
    assert_equal 10, locations.length

    locations.each do |job_name, index|
      workflow = d171_workflow
      workflow.dig("jobs", job_name, "steps", index)["uses"] = "actions/checkout@v6"
      assert_d171_routing_rejected(
        workflow,
        "mutable checkout in #{job_name}",
        expected_context: "checkout pin"
      )

      workflow = d171_workflow
      workflow.dig("jobs", job_name, "steps", index, "with")["persist-credentials"] = true
      assert_d171_routing_rejected(
        workflow,
        "persisted checkout credentials in #{job_name}",
        expected_context: "checkout credentials"
      )
    end
  end

  def test_d171_rejects_classifier_permission_checkout_and_history_drift
    mutations = {
      "write permission" => lambda do |workflow|
        workflow.dig("jobs", "classify-changes", "permissions")["contents"] = "write"
      end,
      "shallow checkout" => lambda do |workflow|
        checkout = workflow.dig("jobs", "classify-changes", "steps").first
        checkout.fetch("with")["fetch-depth"] = 1
      end,
      "inexact checkout ref" => lambda do |workflow|
        checkout = workflow.dig("jobs", "classify-changes", "steps").first
        checkout.fetch("with")["ref"] = "${{ github.sha }}"
      end
    }

    mutations.each do |label, mutate|
      workflow = d171_workflow
      mutate.call(workflow)
      assert_d171_routing_rejected(
        workflow,
        label,
        expected_context: "classify-changes"
      )
    end
  end

  def test_d171_rejects_each_classifier_base_or_head_binding_mutation
    expected = {
      "PR_BASE_SHA" => "${{ github.event.pull_request.base.sha }}",
      "PR_HEAD_SHA" => "${{ github.event.pull_request.head.sha }}",
      "PUSH_BASE_SHA" => "${{ github.event.before }}",
      "PUSH_HEAD_SHA" => "${{ github.sha }}"
    }

    expected.each_key do |variable|
      workflow = d171_workflow
      classify = workflow.dig("jobs", "classify-changes", "steps").find do |step|
        step["id"] == "classify"
      end
      classify.fetch("env")[variable] = "${{ github.ref }}"
      assert_d171_routing_rejected(
        workflow,
        "classifier #{variable}",
        expected_context: "classify-changes job"
      )
    end
  end

  def test_d171_rejects_classifier_diff_range_or_no_renames_drift
    {
      "rename detection enabled" => ["git diff --no-renames", "git diff"],
      "reversed diff range" => [
        '-z "$base_sha" "$head_sha"',
        '-z "$head_sha" "$base_sha"'
      ]
    }.each do |label, (before, after)|
      workflow = d171_workflow
      classify = workflow.dig("jobs", "classify-changes", "steps").find do |step|
        step["id"] == "classify"
      end
      classify["run"] = classify.fetch("run").sub(before, after)
      assert_d171_routing_rejected(
        workflow,
        label,
        expected_context: "classify-changes job"
      )
    end
  end

  def test_d171_rejects_each_missing_or_misdirected_classifier_output
    %w[compiler pages agent].each do |output|
      workflow = d171_workflow
      workflow.dig("jobs", "classify-changes", "outputs").delete(output)
      assert_d171_routing_rejected(
        workflow,
        "missing #{output} output",
        expected_context: "classify-changes outputs"
      )

      workflow = d171_workflow
      wrong_output = output == "compiler" ? "pages" : "compiler"
      workflow.dig("jobs", "classify-changes", "outputs")[output] =
        "${{ steps.classify.outputs.#{wrong_output} }}"
      assert_d171_routing_rejected(
        workflow,
        "misdirected #{output} output",
        expected_context: "classify-changes outputs"
      )
    end
  end

  def test_d171_requires_cancellation_compatible_governance_and_classifier_dependency
    ["${{ always() }}", "${{ !failure() }}"].each do |condition|
      workflow = d171_workflow
      workflow.dig("jobs", "governance")["if"] = condition
      assert_d171_routing_rejected(
        workflow,
        "governance condition #{condition}",
        expected_context: "governance cancellation condition"
      )
    end

    workflow = d171_workflow
    workflow.dig("jobs", "governance").delete("needs")
    assert_d171_routing_rejected(
      workflow,
      "governance classifier dependency",
      expected_context: "governance dependency"
    )
  end

  def test_d171_requires_every_optional_job_condition_and_classifier_dependency
    {
      "compiler" => D171_COMPILER_JOBS,
      "pages" => D171_PAGES_JOBS
    }.each do |output, jobs|
      jobs.each do |job_name|
        workflow = d171_workflow
        workflow.dig("jobs", job_name)["if"] =
          "needs.classify-changes.outputs.#{output} != 'false'"
        assert_d171_routing_rejected(
          workflow,
          "#{job_name} condition",
          expected_context: "#{job_name} condition"
        )

        workflow = d171_workflow
        job = workflow.dig("jobs", job_name)
        if job["needs"].is_a?(Array)
          job["needs"].delete("classify-changes")
        else
          job.delete("needs")
        end
        assert_d171_routing_rejected(
          workflow,
          "#{job_name} classifier dependency",
          expected_context: "#{job_name} dependency"
        )
      end
    end
  end

  def test_d171_ci_gate_requires_classifier_and_governance_dependencies
    %w[classify-changes governance].each do |dependency|
      workflow = d171_workflow
      workflow.dig("jobs", "ci-gate", "needs").delete(dependency)
      assert_d171_routing_rejected(
        workflow,
        "ci-gate #{dependency} dependency",
        expected_context: "ci-gate needs"
      )
    end
  end

  def test_d171_ci_gate_requires_classifier_and_governance_success
    %w[classify-changes governance].each do |job_name|
      workflow = d171_workflow
      gate = workflow.dig("jobs", "ci-gate", "steps").first
      gate["if"] = gate.fetch("if").sub(
        "needs.#{job_name}.result != 'success'",
        "needs.#{job_name}.result == 'failure'"
      )
      assert_d171_routing_rejected(
        workflow,
        "ci-gate #{job_name} success",
        expected_context: "ci-gate truth table"
      )
    end
  end

  def test_d171_ci_gate_rejects_each_missing_selected_or_unselected_result_branch
    {
      "compiler" => D171_COMPILER_JOBS,
      "pages" => D171_PAGES_JOBS
    }.each do |output, jobs|
      jobs.each do |job_name|
        workflow = d171_workflow
        gate = workflow.dig("jobs", "ci-gate", "steps").first
        gate["if"] = gate.fetch("if").sub(
          "needs.#{job_name}.result != 'success'",
          "needs.#{job_name}.result != 'skipped'"
        )
        assert_d171_routing_rejected(
          workflow,
          "selected #{job_name} result",
          expected_context: "ci-gate truth table"
        )

        workflow = d171_workflow
        gate = workflow.dig("jobs", "ci-gate", "steps").first
        gate["if"] = gate.fetch("if").sub(
          "needs.#{job_name}.result != 'skipped'",
          "needs.#{job_name}.result != 'success'"
        )
        assert_d171_routing_rejected(
          workflow,
          "unselected #{job_name} result",
          expected_context: "ci-gate truth table"
        )
      end
    end
  end

  def test_d171_ci_gate_rejects_malformed_or_missing_output_acceptance
    %w[compiler pages agent].each do |output|
      workflow = d171_workflow
      gate = workflow.dig("jobs", "ci-gate", "steps").first
      original = gate.fetch("if")
      guard = %r{\s*\|\|\s*\(needs\.classify-changes\.outputs\.#{output} != 'true'\s*&&\s*needs\.classify-changes\.outputs\.#{output} != 'false'\)}
      gate["if"] = original.sub(
        guard,
        ""
      )
      refute_equal original, gate["if"], "missing #{output} guard mutation must apply"
      assert_d171_routing_rejected(
        workflow,
        "malformed #{output} output",
        expected_context: "ci-gate truth table"
      )
    end
  end

  def test_d171_concurrency_cancels_prs_only_and_never_main
    mutations = {
      "PR cancellation disabled" => lambda do |workflow|
        workflow.fetch("concurrency")["cancel-in-progress"] = false
      end,
      "main cancellation enabled" => lambda do |workflow|
        workflow.fetch("concurrency")["cancel-in-progress"] = "${{ true }}"
      end,
      "shared concurrency group" => lambda do |workflow|
        workflow.fetch("concurrency")["group"] = "ci-${{ github.ref }}"
      end
    }

    mutations.each do |label, mutate|
      workflow = d171_workflow
      mutate.call(workflow)
      assert_d171_routing_rejected(
        workflow,
        label,
        expected_context: "concurrency"
      )
    end
  end

  def test_d171_delegates_coverage_threshold_workspace_and_sandbox_checks
    {
      "line threshold" => ["--fail-under-lines 100", "--fail-under-lines 99"],
      "region threshold" => ["--fail-under-regions 100", "--fail-under-regions 99"],
      "workspace denominator" => ["llvm-cov --workspace", "llvm-cov"],
      "nobody sandbox" => ['sudo -u nobody env -i', 'sudo env -i']
    }.each do |label, (before, after)|
      workflow = d171_workflow
      step = workflow.dig("jobs", "build-test-coverage", "steps").find do |candidate|
        candidate["name"] == COVERAGE_STEP
      end
      step["run"] = step.fetch("run").sub(before, after)
      assert_d171_routing_rejected(
        workflow,
        label,
        expected_context: "coverage"
      )
    end
  end

  def test_d171_rejects_each_removed_tier1_matrix_leg
    4.times do |index|
      workflow = d171_workflow
      workflow.dig("jobs", "native-build-test", "strategy", "matrix", "include")
              .delete_at(index)
      assert_d171_routing_rejected(
        workflow,
        "matrix leg #{index}",
        expected_context: "Tier-1"
      )
    end
  end

  def test_d171_delegates_performance_provenance_validation
    workflow = d171_workflow
    checkout = workflow.dig("jobs", "frontend-perf-measure", "steps").find do |step|
      step["name"] == "Check out candidate"
    end
    checkout.fetch("with")["ref"] = "${{ github.event.pull_request.head.sha }}"

    assert_d171_routing_rejected(
      workflow,
      "candidate performance provenance",
      expected_context: "source-aware measurement job"
    )
  end

  def test_d171_rejects_pages_gate_removal_and_reviewed_body_drift
    D171_PAGES_JOBS.each do |job_name|
      workflow = d171_workflow
      workflow.fetch("jobs").delete(job_name)
      assert_d171_routing_rejected(
        workflow,
        "missing #{job_name}",
        expected_context: job_name
      )
    end

    {
      "pages-performance" => "Run hermetic Lighthouse pages performance budget gate",
      "pages-accessibility" => "Run hermetic Lighthouse accessibility gate"
    }.each do |job_name, step_name|
      workflow = d171_workflow
      workflow.dig("jobs", job_name, "steps").reject! do |step|
        step["name"] == step_name
      end
      assert_d171_routing_rejected(
        workflow,
        "missing #{job_name} command",
        expected_context: "#{job_name} body"
      )
    end
  end

  def test_d171_ast_round_trip_preserves_triggers_and_is_accepted
    workflow = d171_workflow

    assert_equal(
      {
        "push" => { "branches" => ["main"] },
        "pull_request" => ""
      },
      workflow["on"]
    )
    assert validate_d171_ci_routing(workflow.to_yaml, "ci-d171-round-trip.yml")
  end

  def test_d171_rejects_trigger_widening_and_workflow_dispatch
    mutations = {
      "workflow dispatch" => lambda do |workflow|
        workflow.fetch("on")["workflow_dispatch"] = ""
      end,
      "filtered pull request" => lambda do |workflow|
        workflow.fetch("on")["pull_request"] = { "branches" => ["main"] }
      end,
      "all push branches" => lambda do |workflow|
        workflow.fetch("on")["push"] = ""
      end
    }

    mutations.each do |label, mutate|
      workflow = d171_workflow
      mutate.call(workflow)
      assert_d171_routing_rejected(
        workflow,
        label,
        expected_context: "triggers"
      )
    end
  end

  def test_d171_rejects_extra_top_level_keys_jobs_and_job_keys
    workflow = d171_workflow
    workflow["unexpected-policy"] = "enabled"
    assert_d171_routing_rejected(
      workflow,
      "extra top-level key",
      expected_context: "top-level keys"
    )

    workflow = d171_workflow
    workflow.fetch("jobs")["unexpected-job"] = {
      "runs-on" => "ubuntu-latest",
      "steps" => [{ "run" => "true" }]
    }
    assert_d171_routing_rejected(
      workflow,
      "extra job",
      expected_context: "job set"
    )

    workflow = d171_workflow
    workflow.dig("jobs", "native-build-test")["timeout-minutes"] = "5"
    assert_d171_routing_rejected(
      workflow,
      "extra job key",
      expected_context: "native-build-test body"
    )
  end

  def test_d171_rejects_extra_or_widened_permissions
    workflow = d171_workflow
    workflow.fetch("permissions")["actions"] = "read"
    assert_d171_routing_rejected(
      workflow,
      "extra workflow permission",
      expected_context: "workflow permissions"
    )

    workflow = d171_workflow
    workflow.dig("jobs", "governance", "permissions")["actions"] = "read"
    assert_d171_routing_rejected(
      workflow,
      "extra governance permission",
      expected_context: "governance permissions"
    )

    workflow = d171_workflow
    workflow.dig("jobs", "pages-performance", "permissions")["actions"] = "write"
    assert_d171_routing_rejected(
      workflow,
      "widened Pages permission",
      expected_context: "pages-performance"
    )
  end

  def test_d171_rejects_governance_failure_suppression_and_extra_steps
    workflow = d171_workflow
    workflow.dig("jobs", "governance")["continue-on-error"] = "true"
    assert_d171_routing_rejected(
      workflow,
      "governance continue-on-error",
      expected_context: "governance body"
    )

    workflow = d171_workflow
    workflow.dig("jobs", "governance", "steps") << {
      "name" => "Unexpected policy escape",
      "run" => "true"
    }
    assert_d171_routing_rejected(
      workflow,
      "extra governance step",
      expected_context: "governance body"
    )
  end

  def test_d171_rejects_each_missing_conditioned_or_replaced_governance_policy_body
    policy_steps = [
      "Check agent policy",
      "Check workflow permission policy",
      "Check roadmap evidence",
      "Check README coverage badge binding (issue"
    ]

    policy_steps.each do |step_name|
      workflow = d171_workflow
      removed = workflow.dig("jobs", "governance", "steps").reject! do |step|
        step["name"] == step_name
      end
      refute_nil removed, "missing #{step_name} mutation must apply"
      assert_d171_routing_rejected(
        workflow,
        "missing #{step_name}",
        expected_context: "governance body"
      )

      workflow = d171_workflow
      step = workflow.dig("jobs", "governance", "steps").find do |candidate|
        candidate["name"] == step_name
      end
      step["if"] = "github.event_name == 'push'"
      assert_d171_routing_rejected(
        workflow,
        "conditioned #{step_name}",
        expected_context: "governance body"
      )

      workflow = d171_workflow
      step = workflow.dig("jobs", "governance", "steps").find do |candidate|
        candidate["name"] == step_name
      end
      step["run"] = "true"
      assert_d171_routing_rejected(
        workflow,
        "replaced #{step_name}",
        expected_context: "governance body"
      )
    end
  end

  def test_d171_rejects_matrix_fail_fast_drift
    workflow = d171_workflow
    workflow.dig("jobs", "native-build-test", "strategy")["fail-fast"] = "true"

    assert_d171_routing_rejected(
      workflow,
      "matrix fail-fast",
      expected_context: "Tier-1 strategy"
    )
  end

  def test_d171_rejects_cross_build_artifact_removal_and_cross_verify_weakening
    workflow = d171_workflow
    workflow.dig("jobs", "cross-compile-build", "steps").reject! do |step|
      step["name"] == "Upload the cross-compiled binary"
    end
    assert_d171_routing_rejected(
      workflow,
      "cross artifact upload",
      expected_context: "cross-compile-build body"
    )

    workflow = d171_workflow
    verify = workflow.dig("jobs", "cross-compile-verify", "steps").find do |step|
      step["name"] == "Verify it runs natively and prints the right output"
    end
    original = verify.fetch("run")
    verify["run"] = original.sub(
      'if [ "$output" != "42" ]; then',
      'if false; then'
    )
    refute_equal original, verify["run"]
    assert_d171_routing_rejected(
      workflow,
      "cross verification weakened",
      expected_context: "cross-compile-verify body"
    )
  end

  def test_d171_rejects_case_variant_checkout_action
    workflow = d171_workflow
    checkout = workflow.dig("jobs", "native-build-test", "steps").find do |step|
      step.fetch("uses", "").start_with?("actions/checkout@")
    end
    checkout["uses"] = checkout.fetch("uses").sub("actions/checkout", "Actions/Checkout")

    assert_d171_routing_rejected(
      workflow,
      "case-variant checkout",
      expected_context: "checkout pin"
    )
  end
end
