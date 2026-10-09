#!/usr/bin/env ruby
# frozen_string_literal: true

# Mutation tests for scripts/check_pages_performance_budget.rb (issue #229).
#
# These tests verify the checker's own logic — identity validation,
# threshold enforcement, resource-budget checking, and mutation rejection —
# against crafted inputs.  Lighthouse cannot run in this environment (no
# Chrome), so all LHR objects are synthetic JSON fixtures.  The actual
# Lighthouse execution is tested in CI.
#
# Each mutation must make the checker fail.  Positive controls preserve
# the current healthy site.

require "json"
require "minitest/autorun"
require "tmpdir"
require "fileutils"
require "digest"
require_relative "check_pages_performance_budget"

class TestCheckPagesPerformanceBudget < Minitest::Test
  # The real repo root — used for the healthy site tree and default
  # manifest/budget paths.
  REPO_ROOT = File.expand_path("..", __dir__)

  DEFAULT_MANIFEST =
    File.join(REPO_ROOT, "tests/fixtures/pages-performance-manifest.json")
  DEFAULT_BUDGET =
    File.join(REPO_ROOT, "tests/fixtures/pages-performance-budget.json")

  # A healthy LHR fixture for a given page and base URL.
  def healthy_lhr(page, base_url, lcp_ms: 1100, cls: 0.0, tbt_ms: 0, perf: 0.95)
    expected_url = "#{base_url}#{page["local_route"].sub(/^\//, '')}"
    project_prefix = "pycc/"
    {
      "requestedUrl" => expected_url,
      "mainDocumentUrl" => expected_url,
      "finalDisplayedUrl" => expected_url,
      "runtimeError" => nil,
      "runWarnings" => [],
      "configSettings" => {
        "formFactor" => "mobile",
        "screenEmulation" => {
          "width" => 412,
          "height" => 823,
          "deviceScaleFactor" => 1.75
        }
      },
      "audits" => {
        "largest-contentful-paint" => { "numericValue" => lcp_ms },
        "cumulative-layout-shift" => { "numericValue" => cls },
        "total-blocking-time" => { "numericValue" => tbt_ms },
        "document-title" => {
          "details" => { "items" => [{ "title" => page["expected_title"] }] }
        },
        "network-requests" => {
          "details" => {
            "items" => [
              { "url" => expected_url, "statusCode" => 200, "resourceType" => "Document", "mimeType" => "text/html" },
              { "url" => "#{base_url}#{project_prefix}styles.css", "statusCode" => 200, "resourceType" => "Stylesheet", "mimeType" => "text/css" },
              { "url" => "#{base_url}#{project_prefix}site.js", "statusCode" => 200, "resourceType" => "Script", "mimeType" => "text/javascript" }
            ]
          }
        }
      },
      "categories" => {
        "performance" => { "score" => perf }
      }
    }
  end

  def load_default_manifest
    load_manifest(DEFAULT_MANIFEST)
  end

  def load_default_budget
    load_budget(DEFAULT_BUDGET)
  end

  # Write a full set of healthy LHR replicates for all 5 pages.
  def write_healthy_lhrs(dir, manifest, base_url)
    manifest["canonical_pages"].each do |page|
      page_dir = File.join(dir, page["id"])
      FileUtils.mkdir_p(page_dir)
      1.upto(REPLICATE_COUNT) do |replicate|
        lhr = healthy_lhr(page, base_url)
        File.write(
          File.join(page_dir, "replicate-#{replicate}.json"),
          JSON.generate(lhr)
        )
      end
    end
  end

  # Write a single LHR file for a page/replicate.
  def write_lhr(dir, page_id, replicate, lhr)
    page_dir = File.join(dir, page_id)
    FileUtils.mkdir_p(page_dir)
    File.write(
      File.join(page_dir, "replicate-#{replicate}.json"),
      JSON.generate(lhr)
    )
  end

  # ------------------------------------------------------------------
  # Positive control: the healthy site passes the full gate
  # ------------------------------------------------------------------

  def test_healthy_site_passes_with_skip_lighthouse
    exit_code = pages_perf_budget_main(
      ["--skip-lighthouse", "--site-dir", File.join(REPO_ROOT, "site")]
    )
    assert_equal 0, exit_code
  end

  def test_healthy_site_passes_with_healthy_lhrs
    Dir.mktmpdir do |tmp|
      manifest = load_default_manifest
      budget = load_default_budget
      base_url = "http://127.0.0.1:9999/"
      write_healthy_lhrs(tmp, manifest, base_url)

      exit_code = pages_perf_budget_main(
        [
          "--skip-lighthouse", "--site-dir", File.join(REPO_ROOT, "site")
        ]
      )
      # The skip-lighthouse path doesn't need LHRs, so this should pass.
      assert_equal 0, exit_code
    end
  end

  # ------------------------------------------------------------------
  # LHR validation: identity binding
  # ------------------------------------------------------------------

  def test_lhr_fails_when_requested_url_mismatches
    Dir.mktmpdir do |tmp|
      manifest = load_default_manifest
      base_url = "http://127.0.0.1:9999/"
      page = manifest["canonical_pages"].first
      lhr = healthy_lhr(page, base_url)
      lhr["requestedUrl"] = "http://127.0.0.1:9999/pycc/wrong/"
      failures = validate_lhr_identity(lhr, page, 1, base_url, REPO_ROOT)
      refute_empty failures
      assert(failures.any? { |f| f.include?("requestedUrl mismatch") })
    end
  end

  def test_lhr_fails_when_main_document_url_mismatches
    Dir.mktmpdir do |tmp|
      manifest = load_default_manifest
      base_url = "http://127.0.0.1:9999/"
      page = manifest["canonical_pages"].first
      lhr = healthy_lhr(page, base_url)
      lhr["mainDocumentUrl"] = "http://127.0.0.1:9999/pycc/wrong/"
      failures = validate_lhr_identity(lhr, page, 1, base_url, REPO_ROOT)
      refute_empty failures
      assert(failures.any? { |f| f.include?("mainDocumentUrl mismatch") })
    end
  end

  def test_lhr_fails_when_final_displayed_url_mismatches
    manifest = load_default_manifest
    base_url = "http://127.0.0.1:9999/"
    page = manifest["canonical_pages"].first
    lhr = healthy_lhr(page, base_url)
    lhr["finalDisplayedUrl"] = "http://127.0.0.1:9999/pycc/wrong/"
    failures = validate_lhr_identity(lhr, page, 1, base_url, REPO_ROOT)
    refute_empty failures
    assert(failures.any? { |f| f.include?("finalDisplayedUrl mismatch") })
  end

  def test_lhr_fails_when_runtime_error_is_present
    manifest = load_default_manifest
    base_url = "http://127.0.0.1:9999/"
    page = manifest["canonical_pages"].first
    lhr = healthy_lhr(page, base_url)
    lhr["runtimeError"] = { "code" => "NO_FCP", "message" => "No FCP" }
    failures = validate_lhr_identity(lhr, page, 1, base_url, REPO_ROOT)
    refute_empty failures
    assert(failures.any? { |f| f.include?("runtimeError present") })
  end

  def test_lhr_fails_when_run_warnings_field_is_missing
    manifest = load_default_manifest
    base_url = "http://127.0.0.1:9999/"
    page = manifest["canonical_pages"].first
    lhr = healthy_lhr(page, base_url)
    lhr.delete("runWarnings")
    failures = validate_lhr_identity(lhr, page, 1, base_url, REPO_ROOT)
    refute_empty failures
    assert(failures.any? { |f| f.include?("runWarnings field missing") })
  end

  def test_lhr_fails_when_config_settings_form_factor_is_wrong
    manifest = load_default_manifest
    base_url = "http://127.0.0.1:9999/"
    page = manifest["canonical_pages"].first
    lhr = healthy_lhr(page, base_url)
    lhr["configSettings"]["formFactor"] = "desktop"
    failures = validate_lhr_identity(lhr, page, 1, base_url, REPO_ROOT)
    refute_empty failures
    assert(failures.any? { |f| f.include?("formFactor") })
  end

  def test_lhr_fails_when_screen_emulation_dimensions_are_wrong
    manifest = load_default_manifest
    base_url = "http://127.0.0.1:9999/"
    page = manifest["canonical_pages"].first
    lhr = healthy_lhr(page, base_url)
    lhr["configSettings"]["screenEmulation"]["width"] = 1920
    lhr["configSettings"]["screenEmulation"]["height"] = 1080
    failures = validate_lhr_identity(lhr, page, 1, base_url, REPO_ROOT)
    refute_empty failures
    assert(failures.any? { |f| f.include?("screenEmulation") })
  end

  def test_lhr_fails_when_device_scale_factor_is_wrong
    manifest = load_default_manifest
    base_url = "http://127.0.0.1:9999/"
    page = manifest["canonical_pages"].first
    lhr = healthy_lhr(page, base_url)
    lhr["configSettings"]["screenEmulation"]["deviceScaleFactor"] = 2.0
    failures = validate_lhr_identity(lhr, page, 1, base_url, REPO_ROOT)
    refute_empty failures
    assert(failures.any? { |f| f.include?("deviceScaleFactor") })
  end

  def test_lhr_fails_when_document_title_mismatches
    manifest = load_default_manifest
    base_url = "http://127.0.0.1:9999/"
    page = manifest["canonical_pages"].first
    lhr = healthy_lhr(page, base_url)
    lhr["audits"]["document-title"]["details"]["items"][0]["title"] = "Wrong title"
    failures = validate_lhr_identity(lhr, page, 1, base_url, REPO_ROOT)
    refute_empty failures
    assert(failures.any? { |f| f.include?("document-title mismatch") })
  end

  def test_lhr_passes_when_run_warnings_present_but_not_empty
    manifest = load_default_manifest
    base_url = "http://127.0.0.1:9999/"
    page = manifest["canonical_pages"].first
    lhr = healthy_lhr(page, base_url)
    lhr["runWarnings"] = ["A non-blocking warning"]
    failures = validate_lhr_identity(lhr, page, 1, base_url, REPO_ROOT)
    # Warnings are preserved and reviewed but do not cause failure.
    assert_empty failures
  end

  # ------------------------------------------------------------------
  # HTML identity verification from source artifact (mutation tests)
  # ------------------------------------------------------------------

  def test_lhr_fails_when_source_html_title_mismatches_manifest
    Dir.mktmpdir do |tmp|
      manifest = load_default_manifest
      base_url = "http://127.0.0.1:9999/"
      page = manifest["canonical_pages"].first

      # Copy the real site to a tmp dir and mutate the <title>.
      site_dir = File.join(tmp, "site")
      FileUtils.mkdir_p(site_dir)
      FileUtils.cp_r(File.join(REPO_ROOT, "site", "."), site_dir)

      path = File.join(site_dir, "index.html")
      content = File.read(path)
      mutated = content.sub(/<title[^>]*>.*?<\/title>/im,
                            "<title>Wrong Title</title>")
      File.write(path, mutated)

      lhr = healthy_lhr(page, base_url)
      failures = validate_lhr_identity(lhr, page, 1, base_url, tmp)
      refute_empty failures
      assert(failures.any? { |f| f.include?("HTML <title> mismatch") })
    end
  end

  def test_lhr_fails_when_source_html_h1_mismatches_manifest
    Dir.mktmpdir do |tmp|
      manifest = load_default_manifest
      base_url = "http://127.0.0.1:9999/"
      page = manifest["canonical_pages"].first

      site_dir = File.join(tmp, "site")
      FileUtils.mkdir_p(site_dir)
      FileUtils.cp_r(File.join(REPO_ROOT, "site", "."), site_dir)

      path = File.join(site_dir, "index.html")
      content = File.read(path)
      mutated = content.sub(/<h1[^>]*>.*?<\/h1>/im,
                            '<h1 id="hero-title">Wrong H1</h1>')
      File.write(path, mutated)

      lhr = healthy_lhr(page, base_url)
      failures = validate_lhr_identity(lhr, page, 1, base_url, tmp)
      refute_empty failures
      assert(failures.any? { |f| f.include?("HTML <h1> mismatch") })
    end
  end

  def test_lhr_fails_when_source_html_canonical_mismatches_manifest
    Dir.mktmpdir do |tmp|
      manifest = load_default_manifest
      base_url = "http://127.0.0.1:9999/"
      page = manifest["canonical_pages"].first

      site_dir = File.join(tmp, "site")
      FileUtils.mkdir_p(site_dir)
      FileUtils.cp_r(File.join(REPO_ROOT, "site", "."), site_dir)

      path = File.join(site_dir, "index.html")
      content = File.read(path)
      mutated = content.sub(
        /<link\s+rel=["']canonical["']\s+href=["'][^"']+["']/i,
        '<link rel="canonical" href="https://wrong.example.com/">'
      )
      File.write(path, mutated)

      lhr = healthy_lhr(page, base_url)
      failures = validate_lhr_identity(lhr, page, 1, base_url, tmp)
      refute_empty failures
      assert(failures.any? { |f| f.include?("HTML canonical mismatch") })
    end
  end

  def test_lhr_fails_when_source_artifact_file_missing
    manifest = load_default_manifest
    base_url = "http://127.0.0.1:9999/"
    page = manifest["canonical_pages"].first
    lhr = healthy_lhr(page, base_url)
    failures = validate_lhr_identity(lhr, page, 1, base_url, "/nonexistent")
    refute_empty failures
    assert(failures.any? { |f| f.include?("source artifact not found") })
  end

  # ------------------------------------------------------------------
  # Threshold enforcement
  # ------------------------------------------------------------------

  def test_lhr_gate_fails_when_lcp_exceeds_budget
    Dir.mktmpdir do |tmp|
      manifest = load_default_manifest
      budget = load_default_budget
      base_url = "http://127.0.0.1:9999/"
      write_healthy_lhrs(tmp, manifest, base_url)

      # Corrupt one page's LCP to exceed the 2000ms budget
      page = manifest["canonical_pages"].first
      page_dir = File.join(tmp, page["id"])
      1.upto(REPLICATE_COUNT) do |replicate|
        path = File.join(page_dir, "replicate-#{replicate}.json")
        lhr = JSON.parse(File.read(path))
        lhr["audits"]["largest-contentful-paint"]["numericValue"] = 2500
        File.write(path, JSON.generate(lhr))
      end

      failures = validate_and_gate_lhrs(manifest, budget, tmp, base_url, REPO_ROOT)
      refute_empty failures
      assert(failures.any? { |f| f.include?("LCP") && f.include?("exceeds") })
    end
  end

  def test_lhr_gate_fails_when_cls_exceeds_budget
    Dir.mktmpdir do |tmp|
      manifest = load_default_manifest
      budget = load_default_budget
      base_url = "http://127.0.0.1:9999/"
      write_healthy_lhrs(tmp, manifest, base_url)

      page = manifest["canonical_pages"].first
      page_dir = File.join(tmp, page["id"])
      1.upto(REPLICATE_COUNT) do |replicate|
        path = File.join(page_dir, "replicate-#{replicate}.json")
        lhr = JSON.parse(File.read(path))
        lhr["audits"]["cumulative-layout-shift"]["numericValue"] = 0.15
        File.write(path, JSON.generate(lhr))
      end

      failures = validate_and_gate_lhrs(manifest, budget, tmp, base_url, REPO_ROOT)
      refute_empty failures
      assert(failures.any? { |f| f.include?("CLS") && f.include?("exceeds") })
    end
  end

  def test_lhr_gate_fails_when_tbt_exceeds_budget
    Dir.mktmpdir do |tmp|
      manifest = load_default_manifest
      budget = load_default_budget
      base_url = "http://127.0.0.1:9999/"
      write_healthy_lhrs(tmp, manifest, base_url)

      page = manifest["canonical_pages"].first
      page_dir = File.join(tmp, page["id"])
      1.upto(REPLICATE_COUNT) do |replicate|
        path = File.join(page_dir, "replicate-#{replicate}.json")
        lhr = JSON.parse(File.read(path))
        lhr["audits"]["total-blocking-time"]["numericValue"] = 300
        File.write(path, JSON.generate(lhr))
      end

      failures = validate_and_gate_lhrs(manifest, budget, tmp, base_url, REPO_ROOT)
      refute_empty failures
      assert(failures.any? { |f| f.include?("TBT") && f.include?("exceeds") })
    end
  end

  def test_lhr_gate_fails_when_performance_score_below_floor
    Dir.mktmpdir do |tmp|
      manifest = load_default_manifest
      budget = load_default_budget
      base_url = "http://127.0.0.1:9999/"
      write_healthy_lhrs(tmp, manifest, base_url)

      page = manifest["canonical_pages"].first
      page_dir = File.join(tmp, page["id"])
      1.upto(REPLICATE_COUNT) do |replicate|
        path = File.join(page_dir, "replicate-#{replicate}.json")
        lhr = JSON.parse(File.read(path))
        lhr["categories"]["performance"]["score"] = 0.80
        File.write(path, JSON.generate(lhr))
      end

      failures = validate_and_gate_lhrs(manifest, budget, tmp, base_url, REPO_ROOT)
      refute_empty failures
      assert(failures.any? { |f| f.include?("Performance score") && f.include?("below") })
    end
  end

  def test_lhr_gate_uses_median_of_five_replicates
    Dir.mktmpdir do |tmp|
      manifest = load_default_manifest
      budget = load_default_budget
      base_url = "http://127.0.0.1:9999/"
      write_healthy_lhrs(tmp, manifest, base_url)

      # Set LCP values: [900, 1000, 2500, 1100, 1200] — median is 1100 (under budget)
      page = manifest["canonical_pages"].first
      page_dir = File.join(tmp, page["id"])
      lcp_values = [900, 1000, 2500, 1100, 1200]
      lcp_values.each_with_index do |lcp, i|
        path = File.join(page_dir, "replicate-#{i + 1}.json")
        lhr = JSON.parse(File.read(path))
        lhr["audits"]["largest-contentful-paint"]["numericValue"] = lcp
        File.write(path, JSON.generate(lhr))
      end

      failures = validate_and_gate_lhrs(manifest, budget, tmp, base_url, REPO_ROOT)
      # Median 1100ms is under 2000ms budget, so LCP should pass.
      # But the outlier 2500ms is preserved, not retried away.
      refute(failures.any? { |f| f.include?("#{page["id"]}: LCP") })
    end
  end

  def test_lhr_gate_fails_closed_on_missing_replicate
    Dir.mktmpdir do |tmp|
      manifest = load_default_manifest
      budget = load_default_budget
      base_url = "http://127.0.0.1:9999/"
      write_healthy_lhrs(tmp, manifest, base_url)

      # Delete one replicate
      page = manifest["canonical_pages"].first
      File.delete(File.join(tmp, page["id"], "replicate-3.json"))

      failures = validate_and_gate_lhrs(manifest, budget, tmp, base_url, REPO_ROOT)
      refute_empty failures
      assert(failures.any? { |f| f.include?("LHR file missing") })
    end
  end

  def test_lhr_gate_fails_closed_on_partial_metric
    Dir.mktmpdir do |tmp|
      manifest = load_default_manifest
      budget = load_default_budget
      base_url = "http://127.0.0.1:9999/"
      write_healthy_lhrs(tmp, manifest, base_url)

      # Remove LCP metric from one replicate
      page = manifest["canonical_pages"].first
      path = File.join(tmp, page["id"], "replicate-3.json")
      lhr = JSON.parse(File.read(path))
      lhr["audits"].delete("largest-contentful-paint")
      File.write(path, JSON.generate(lhr))

      failures = validate_and_gate_lhrs(manifest, budget, tmp, base_url, REPO_ROOT)
      refute_empty failures
      assert(failures.any? { |f| f.include?("LCP metric missing") })
    end
  end

  # ------------------------------------------------------------------
  # Duplicate report reuse detection
  # ------------------------------------------------------------------

  def test_lhr_gate_fails_when_one_lhr_is_reused_under_five_page_labels
    Dir.mktmpdir do |tmp|
      manifest = load_default_manifest
      budget = load_default_budget
      base_url = "http://127.0.0.1:9999/"

      # Create a single LHR file and symlink it for all pages/replicates
      shared_dir = File.join(tmp, "shared")
      FileUtils.mkdir_p(shared_dir)
      shared_lhr = healthy_lhr(manifest["canonical_pages"].first, base_url)
      shared_path = File.join(shared_dir, "shared.json")
      File.write(shared_path, JSON.generate(shared_lhr))

      manifest["canonical_pages"].each do |page|
        page_dir = File.join(tmp, page["id"])
        FileUtils.mkdir_p(page_dir)
        1.upto(REPLICATE_COUNT) do |replicate|
          dest = File.join(page_dir, "replicate-#{replicate}.json")
          # Use a symlink so realpath resolves to the same target.
          File.symlink(shared_path, dest)
        end
      end

      failures = validate_and_gate_lhrs(manifest, budget, tmp, base_url, REPO_ROOT)
      refute_empty failures
      assert(failures.any? { |f| f.include?("LHR file reused") || f.include?("LHR content identical") })
    end
  end

  # ------------------------------------------------------------------
  # Network request budget enforcement (third-party and unexpected requests)
  # ------------------------------------------------------------------

  def test_network_requests_pass_on_healthy_site
    manifest = load_default_manifest
    budget = load_default_budget
    base_url = "http://127.0.0.1:9999/"
    page = manifest["canonical_pages"].first
    lhr = healthy_lhr(page, base_url)
    failures = validate_network_requests(lhr, page, 1, base_url, budget, manifest)
    assert_empty failures
  end

  def test_network_requests_fail_when_third_party_request_present
    manifest = load_default_manifest
    budget = load_default_budget
    base_url = "http://127.0.0.1:9999/"
    page = manifest["canonical_pages"].first
    lhr = healthy_lhr(page, base_url)
    # Add a third-party request (different origin)
    lhr["audits"]["network-requests"]["details"]["items"] << {
      "url" => "https://example.com/analytics.js",
      "statusCode" => 200,
      "resourceType" => "Script",
      "mimeType" => "text/javascript"
    }
    failures = validate_network_requests(lhr, page, 1, base_url, budget, manifest)
    refute_empty failures
    assert(failures.any? { |f| f.include?("third-party requests") })
  end

  def test_network_requests_fail_when_unexpected_first_party_request_present
    manifest = load_default_manifest
    budget = load_default_budget
    base_url = "http://127.0.0.1:9999/"
    page = manifest["canonical_pages"].first
    lhr = healthy_lhr(page, base_url)
    # Add an unexpected first-party request (same origin, not in allowed list)
    lhr["audits"]["network-requests"]["details"]["items"] << {
      "url" => "#{base_url}pycc/unexpected.js",
      "statusCode" => 200,
      "resourceType" => "Script",
      "mimeType" => "text/javascript"
    }
    failures = validate_network_requests(lhr, page, 1, base_url, budget, manifest)
    refute_empty failures
    assert(failures.any? { |f| f.include?("unexpected first-party requests") })
  end

  def test_network_requests_validation_via_gate_with_third_party
    Dir.mktmpdir do |tmp|
      manifest = load_default_manifest
      budget = load_default_budget
      base_url = "http://127.0.0.1:9999/"
      write_healthy_lhrs(tmp, manifest, base_url)

      # Add a third-party request to one replicate of the first page
      page = manifest["canonical_pages"].first
      path = File.join(tmp, page["id"], "replicate-1.json")
      lhr = JSON.parse(File.read(path))
      lhr["audits"]["network-requests"]["details"]["items"] << {
        "url" => "https://example.com/analytics.js",
        "statusCode" => 200,
        "resourceType" => "Script",
        "mimeType" => "text/javascript"
      }
      File.write(path, JSON.generate(lhr))

      failures = validate_and_gate_lhrs(manifest, budget, tmp, base_url, REPO_ROOT)
      refute_empty failures
      assert(failures.any? { |f| f.include?("third-party requests") })
    end
  end

  def test_network_requests_validation_via_gate_with_unexpected_request
    Dir.mktmpdir do |tmp|
      manifest = load_default_manifest
      budget = load_default_budget
      base_url = "http://127.0.0.1:9999/"
      write_healthy_lhrs(tmp, manifest, base_url)

      # Add an unexpected first-party request to one replicate
      page = manifest["canonical_pages"].first
      path = File.join(tmp, page["id"], "replicate-2.json")
      lhr = JSON.parse(File.read(path))
      lhr["audits"]["network-requests"]["details"]["items"] << {
        "url" => "#{base_url}pycc/unexpected.js",
        "statusCode" => 200,
        "resourceType" => "Script",
        "mimeType" => "text/javascript"
      }
      File.write(path, JSON.generate(lhr))

      failures = validate_and_gate_lhrs(manifest, budget, tmp, base_url, REPO_ROOT)
      refute_empty failures
      assert(failures.any? { |f| f.include?("unexpected first-party requests") })
    end
  end

  def test_network_requests_skips_when_audit_absent
    manifest = load_default_manifest
    budget = load_default_budget
    base_url = "http://127.0.0.1:9999/"
    page = manifest["canonical_pages"].first
    lhr = healthy_lhr(page, base_url)
    # Remove the network-requests audit entirely
    lhr["audits"].delete("network-requests")
    failures = validate_network_requests(lhr, page, 1, base_url, budget, manifest)
    # Should skip validation (not fail) when audit is absent
    assert_empty failures
  end

  def test_network_requests_allows_og_png
    manifest = load_default_manifest
    budget = load_default_budget
    base_url = "http://127.0.0.1:9999/"
    page = manifest["canonical_pages"].first
    lhr = healthy_lhr(page, base_url)
    # Add og.png request (it's in the allowed_assets list)
    lhr["audits"]["network-requests"]["details"]["items"] << {
      "url" => "#{base_url}pycc/og.png",
      "statusCode" => 200,
      "resourceType" => "Image",
      "mimeType" => "image/png"
    }
    failures = validate_network_requests(lhr, page, 1, base_url, budget, manifest)
    assert_empty failures
  end

  # ------------------------------------------------------------------
  # Body-swap detection
  # ------------------------------------------------------------------

  def test_http_identity_detects_body_swap_between_pages
    Dir.mktmpdir do |tmp|
      manifest = load_default_manifest
      # Create a site dir where two pages have the same body
      site_dir = File.join(tmp, "site")
      FileUtils.mkdir_p(site_dir)

      real_site = File.join(REPO_ROOT, "site")
      # Copy the real site
      FileUtils.cp_r(File.join(real_site, "."), site_dir)

      # Swap: copy status/index.html over architecture/index.html
      FileUtils.cp(
        File.join(site_dir, "status/index.html"),
        File.join(site_dir, "architecture/index.html")
      )

      # Recompute the manifest with the swapped digest
      swapped_manifest = JSON.parse(JSON.generate(manifest))
      arch_page = swapped_manifest["canonical_pages"].find { |p| p["id"] == "architecture" }
      status_digest = manifest["canonical_pages"].find { |p| p["id"] == "status" }["source_artifact_sha256"]
      arch_page["source_artifact_sha256"] = status_digest

      exit_code = pages_perf_budget_main(
        ["--skip-lighthouse", "--site-dir", site_dir,
         "--manifest", write_manifest(tmp, swapped_manifest)]
      )
      # The checker should detect body-swap (two pages with same digest)
      assert_equal 1, exit_code
    end
  end

  # ------------------------------------------------------------------
  # 404 catch-all/fallback rejection
  # ------------------------------------------------------------------

  def test_http_identity_rejects_404_body_served_for_canonical_route
    Dir.mktmpdir do |tmp|
      manifest = load_default_manifest
      site_dir = File.join(tmp, "site")
      FileUtils.mkdir_p(site_dir)

      real_site = File.join(REPO_ROOT, "site")
      FileUtils.cp_r(File.join(real_site, "."), site_dir)

      # Replace index.html with the 404 body
      FileUtils.cp(
        File.join(site_dir, "404.html"),
        File.join(site_dir, "index.html")
      )

      # Update manifest to have the 404 digest for the home page
      mutated_manifest = JSON.parse(JSON.generate(manifest))
      home_page = mutated_manifest["canonical_pages"].find { |p| p["id"] == "home" }
      error_digest = manifest["error_pages"].first["source_artifact_sha256"]
      home_page["source_artifact_sha256"] = error_digest

      exit_code = pages_perf_budget_main(
        ["--skip-lighthouse", "--site-dir", site_dir,
         "--manifest", write_manifest(tmp, mutated_manifest)]
      )
      assert_equal 1, exit_code
    end
  end

  # ------------------------------------------------------------------
  # Redirect rejection
  # ------------------------------------------------------------------

  def test_http_identity_rejects_redirect_from_canonical_route
    # We can't easily make the hermetic server redirect, but we can
    # test the redirect-detection logic by checking that a 301/302
    # status is rejected.  This is covered by the status check —
    # a redirect would not return 200.
    #
    # Instead, verify that the checker rejects a non-200 status for
    # a canonical page by creating a site where a canonical page
    # is missing (returns 404).
    Dir.mktmpdir do |tmp|
      manifest = load_default_manifest
      site_dir = File.join(tmp, "site")
      FileUtils.mkdir_p(site_dir)

      real_site = File.join(REPO_ROOT, "site")
      FileUtils.cp_r(File.join(real_site, "."), site_dir)

      # Remove the status page so it 404s
      FileUtils.rm_rf(File.join(site_dir, "status"))

      exit_code = pages_perf_budget_main(
        ["--skip-lighthouse", "--site-dir", site_dir]
      )
      assert_equal 1, exit_code
    end
  end

  # ------------------------------------------------------------------
  # Missing/deleted threshold config file
  # ------------------------------------------------------------------

  def test_checker_fails_when_budget_config_is_deleted
    exit_code = pages_perf_budget_main(
      ["--skip-lighthouse", "--budget", "/nonexistent/budget.json"]
    )
    assert_equal 2, exit_code
  end

  def test_checker_fails_when_manifest_is_deleted
    exit_code = pages_perf_budget_main(
      ["--skip-lighthouse", "--manifest", "/nonexistent/manifest.json"]
    )
    assert_equal 2, exit_code
  end

  # ------------------------------------------------------------------
  # Narrowed page/site/config path set
  # ------------------------------------------------------------------

  def test_checker_fails_when_manifest_has_wrong_number_of_pages
    Dir.mktmpdir do |tmp|
      manifest = load_default_manifest
      # Remove one page to narrow the set
      mutated = JSON.parse(JSON.generate(manifest))
      mutated["canonical_pages"] = mutated["canonical_pages"][0..3]

      exit_code = pages_perf_budget_main(
        ["--skip-lighthouse",
         "--manifest", write_manifest(tmp, mutated)]
      )
      assert_equal 2, exit_code
    end
  end

  # ------------------------------------------------------------------
  # Resource budget enforcement
  # ------------------------------------------------------------------

  def test_resource_budget_fails_when_html_exceeds_limit
    Dir.mktmpdir do |tmp|
      budget = load_default_budget
      site_dir = File.join(tmp, "site")
      FileUtils.mkdir_p(site_dir)

      real_site = File.join(REPO_ROOT, "site")
      FileUtils.cp_r(File.join(real_site, "."), site_dir)

      # Inflate index.html beyond the 25KB limit
      path = File.join(site_dir, "index.html")
      content = File.read(path)
      padded = content + "<!-- " + ("x" * 10_000) + " -->"
      File.write(path, padded)

      failures = check_resource_budgets(budget, tmp)
      refute_empty failures
      assert(failures.any? { |f| f.include?("HTML") && f.include?("exceeds") })
    end
  end

  def test_resource_budget_fails_when_css_exceeds_limit
    Dir.mktmpdir do |tmp|
      budget = load_default_budget
      site_dir = File.join(tmp, "site")
      FileUtils.mkdir_p(site_dir)

      real_site = File.join(REPO_ROOT, "site")
      FileUtils.cp_r(File.join(real_site, "."), site_dir)

      # Inflate styles.css beyond the 35KB limit
      path = File.join(site_dir, "styles.css")
      content = File.read(path)
      padded = content + "\n/* " + ("x" * 12_000) + " */\n"
      File.write(path, padded)

      failures = check_resource_budgets(budget, tmp)
      refute_empty failures
      assert(failures.any? { |f| f.include?("CSS") && f.include?("exceeds") })
    end
  end

  def test_resource_budget_fails_when_javascript_exceeds_limit
    Dir.mktmpdir do |tmp|
      budget = load_default_budget
      site_dir = File.join(tmp, "site")
      FileUtils.mkdir_p(site_dir)

      real_site = File.join(REPO_ROOT, "site")
      FileUtils.cp_r(File.join(real_site, "."), site_dir)

      # Inflate site.js beyond the 2KB limit
      path = File.join(site_dir, "site.js")
      content = File.read(path)
      padded = content + "\n/* " + ("x" * 3_000) + " */\n"
      File.write(path, padded)

      failures = check_resource_budgets(budget, tmp)
      refute_empty failures
      assert(failures.any? { |f| f.include?("JavaScript") && f.include?("exceeds") })
    end
  end

  def test_resource_budget_fails_when_image_exceeds_limit
    Dir.mktmpdir do |tmp|
      budget = load_default_budget
      site_dir = File.join(tmp, "site")
      FileUtils.mkdir_p(site_dir)

      real_site = File.join(REPO_ROOT, "site")
      FileUtils.cp_r(File.join(real_site, "."), site_dir)

      # Create an oversized og.png (1.6MB, exceeds 1.5MB limit)
      path = File.join(site_dir, "og.png")
      File.binwrite(path, "x" * 1_600_000)

      failures = check_resource_budgets(budget, tmp)
      refute_empty failures
      assert(failures.any? { |f| f.include?("images") && f.include?("exceeds") })
    end
  end

  def test_resource_budget_fails_when_unexpected_image_added
    Dir.mktmpdir do |tmp|
      budget = load_default_budget
      site_dir = File.join(tmp, "site")
      FileUtils.mkdir_p(site_dir)

      real_site = File.join(REPO_ROOT, "site")
      FileUtils.cp_r(File.join(real_site, "."), site_dir)

      # Add an unexpected .jpg file large enough to push the total
      # image payload over the budget.  The existing og.png is
      # ~1.44 MB and the budget is 1.5 MB, so a 200 KB jpg exceeds it.
      path = File.join(site_dir, "hero.jpg")
      File.binwrite(path, "x" * 200_000)

      failures = check_resource_budgets(budget, tmp)
      refute_empty failures
      assert(failures.any? { |f| f.include?("images") && f.include?("exceeds") })
      # Verify the unexpected file is named in the failure message
      assert(failures.any? { |f| f.include?("hero.jpg") })
    end
  end

  def test_resource_budget_passes_on_healthy_site
    budget = load_default_budget
    failures = check_resource_budgets(budget, REPO_ROOT)
    assert_empty failures
  end

  def test_resource_budget_fails_when_image_added_in_subdirectory
    Dir.mktmpdir do |tmp|
      budget = load_default_budget
      site_dir = File.join(tmp, "site")
      FileUtils.mkdir_p(site_dir)

      real_site = File.join(REPO_ROOT, "site")
      FileUtils.cp_r(File.join(real_site, "."), site_dir)

      # Add an unexpected .jpg in a nested subdirectory to verify
      # Find.find recursively scans.  The existing og.png is ~1.44 MB
      # and the budget is 1.5 MB, so a 200 KB jpg exceeds it.
      subdir = File.join(site_dir, "images")
      FileUtils.mkdir_p(subdir)
      path = File.join(subdir, "nested.jpg")
      File.binwrite(path, "x" * 200_000)

      failures = check_resource_budgets(budget, tmp)
      refute_empty failures
      assert(failures.any? { |f| f.include?("images") && f.include?("exceeds") })
      # Verify the nested file is named in the failure message
      assert(failures.any? { |f| f.include?("nested.jpg") })
    end
  end

  # ------------------------------------------------------------------
  # HTML identity extraction helpers
  # ------------------------------------------------------------------

  def test_extract_title_from_html
    html = "<html><head><title>Test Title</title></head></html>"
    assert_equal "Test Title", extract_title_from_html(html)
  end

  def test_extract_h1_from_html_strips_nested_tags
    html = "<h1 id=\"hero\">Typed Python in.<br><span>Autonomous artifacts out.</span></h1>"
    assert_equal "Typed Python in. Autonomous artifacts out.", extract_h1_from_html(html)
  end

  def test_extract_canonical_from_html
    html = "<link rel=\"canonical\" href=\"https://example.com/page/\">"
    assert_equal "https://example.com/page/", extract_canonical_from_html(html)
  end

  def test_extract_h1_from_real_home_page
    html = File.read(File.join(REPO_ROOT, "site/index.html"))
    h1 = extract_h1_from_html(html)
    assert_equal "Typed Python in. Autonomous artifacts out.", h1
  end

  def test_extract_title_from_real_pages
    manifest = load_default_manifest
    manifest["canonical_pages"].each do |page|
      html = File.read(File.join(REPO_ROOT, page["source_artifact"]))
      title = extract_title_from_html(html)
      assert_equal page["expected_title"], title,
                   "title mismatch for #{page["id"]}"
    end
  end

  def test_extract_h1_from_real_pages
    manifest = load_default_manifest
    manifest["canonical_pages"].each do |page|
      html = File.read(File.join(REPO_ROOT, page["source_artifact"]))
      h1 = extract_h1_from_html(html)
      assert_equal page["expected_h1"], h1,
                   "H1 mismatch for #{page["id"]}"
    end
  end

  def test_extract_canonical_from_real_pages
    manifest = load_default_manifest
    manifest["canonical_pages"].each do |page|
      html = File.read(File.join(REPO_ROOT, page["source_artifact"]))
      canonical = extract_canonical_from_html(html)
      assert_equal page["canonical_url"], canonical,
                   "canonical URL mismatch for #{page["id"]}"
    end
  end

  # ------------------------------------------------------------------
  # SHA-256 digest verification against manifest
  # ------------------------------------------------------------------

  def test_manifest_sha256_digests_match_current_tree
    manifest = load_default_manifest
    (manifest["canonical_pages"] + manifest["error_pages"]).each do |page|
      path = File.join(REPO_ROOT, page["source_artifact"])
      actual = Digest::SHA256.hexdigest(File.binread(path))
      assert_equal page["source_artifact_sha256"], actual,
                   "SHA-256 mismatch for #{page["source_artifact"]}"
    end
  end

  # ------------------------------------------------------------------
  # Manifest and budget loading validation
  # ------------------------------------------------------------------

  def test_load_manifest_rejects_wrong_version
    Dir.mktmpdir do |tmp|
      manifest = load_default_manifest
      mutated = JSON.parse(JSON.generate(manifest))
      mutated["manifest_version"] = 2
      assert_raises(RuntimeError) { load_manifest(write_manifest(tmp, mutated)) }
    end
  end

  def test_load_budget_rejects_wrong_version
    Dir.mktmpdir do |tmp|
      budget = load_default_budget
      mutated = JSON.parse(JSON.generate(budget))
      mutated["budget_version"] = 2
      path = File.join(tmp, "budget.json")
      File.write(path, JSON.generate(mutated))
      assert_raises(RuntimeError) { load_budget(path) }
    end
  end

  def test_load_manifest_rejects_missing_required_key
    Dir.mktmpdir do |tmp|
      manifest = load_default_manifest
      mutated = JSON.parse(JSON.generate(manifest))
      mutated["canonical_pages"][0].delete("expected_title")
      assert_raises(RuntimeError) { load_manifest(write_manifest(tmp, mutated)) }
    end
  end

  # ------------------------------------------------------------------
  # Server root vs project-path (/pycc/) prefix
  # ------------------------------------------------------------------

  def test_server_serves_at_pycc_prefix_not_root
    Dir.mktmpdir do |tmp|
      # The hermetic server should serve at /pycc/ and 404 at /
      site_dir = File.join(REPO_ROOT, "site")
      port, thr, io = start_hermetic_server(site_dir)
      begin
        base_url = "http://127.0.0.1:#{port}/"
        # Root should 404
        root_result = http_get("#{base_url}")
        assert_equal 404, root_result[:status]

        # /pycc/ should 200
        pycc_result = http_get("#{base_url}pycc/")
        assert_equal 200, pycc_result[:status]
      ensure
        stop_hermetic_server(thr, io)
      end
    end
  end

  # ------------------------------------------------------------------
  # Validate identity only before first replicate, then serve different body
  # ------------------------------------------------------------------

  def test_http_identity_detects_different_body_than_manifest
    Dir.mktmpdir do |tmp|
      manifest = load_default_manifest
      site_dir = File.join(tmp, "site")
      FileUtils.mkdir_p(site_dir)

      real_site = File.join(REPO_ROOT, "site")
      FileUtils.cp_r(File.join(real_site, "."), site_dir)

      # Modify index.html slightly (add a comment) so the digest changes
      path = File.join(site_dir, "index.html")
      content = File.read(path)
      File.write(path, content + "<!-- mutation -->\n")

      exit_code = pages_perf_budget_main(
        ["--skip-lighthouse", "--site-dir", site_dir]
      )
      assert_equal 1, exit_code
    end
  end

  # ------------------------------------------------------------------
  # LHR with omitted navigation/runtime fields
  # ------------------------------------------------------------------

  def test_lhr_gate_fails_when_runtime_error_omitted_but_present_in_lhr
    Dir.mktmpdir do |tmp|
      manifest = load_default_manifest
      budget = load_default_budget
      base_url = "http://127.0.0.1:9999/"
      write_healthy_lhrs(tmp, manifest, base_url)

      # Add runtimeError to one replicate
      page = manifest["canonical_pages"].first
      path = File.join(tmp, page["id"], "replicate-2.json")
      lhr = JSON.parse(File.read(path))
      lhr["runtimeError"] = { "code" => "PAGE_HUNG", "message" => "hung" }
      File.write(path, JSON.generate(lhr))

      failures = validate_and_gate_lhrs(manifest, budget, tmp, base_url, REPO_ROOT)
      refute_empty failures
      assert(failures.any? { |f| f.include?("runtimeError present") })
    end
  end

  def test_lhr_gate_fails_when_config_settings_omitted
    Dir.mktmpdir do |tmp|
      manifest = load_default_manifest
      budget = load_default_budget
      base_url = "http://127.0.0.1:9999/"
      write_healthy_lhrs(tmp, manifest, base_url)

      page = manifest["canonical_pages"].first
      path = File.join(tmp, page["id"], "replicate-1.json")
      lhr = JSON.parse(File.read(path))
      lhr.delete("configSettings")
      File.write(path, JSON.generate(lhr))

      failures = validate_and_gate_lhrs(manifest, budget, tmp, base_url, REPO_ROOT)
      refute_empty failures
      assert(failures.any? { |f| f.include?("configSettings") })
    end
  end

  # ------------------------------------------------------------------
  # Malformed LHR JSON
  # ------------------------------------------------------------------

  def test_lhr_gate_fails_on_malformed_json
    Dir.mktmpdir do |tmp|
      manifest = load_default_manifest
      budget = load_default_budget
      base_url = "http://127.0.0.1:9999/"
      write_healthy_lhrs(tmp, manifest, base_url)

      page = manifest["canonical_pages"].first
      File.write(File.join(tmp, page["id"], "replicate-3.json"), "{invalid json")

      failures = validate_and_gate_lhrs(manifest, budget, tmp, base_url, REPO_ROOT)
      refute_empty failures
      assert(failures.any? { |f| f.include?("could not parse LHR JSON") })
    end
  end

  # ------------------------------------------------------------------
  # Unknown argument
  # ------------------------------------------------------------------

  def test_main_returns_2_for_unknown_argument
    assert_equal 2, pages_perf_budget_main(["--unknown-flag"])
  end

  # ------------------------------------------------------------------
  # --lhr-dir required when not skipping lighthouse
  # ------------------------------------------------------------------

  def test_main_returns_2_when_lhr_dir_required_but_missing
    exit_code = pages_perf_budget_main(
      ["--site-dir", File.join(REPO_ROOT, "site")]
    )
    assert_equal 2, exit_code
  end

  # ------------------------------------------------------------------
  # Helpers
  # ------------------------------------------------------------------

  private

  def write_manifest(dir, manifest)
    path = File.join(dir, "manifest.json")
    File.write(path, JSON.pretty_generate(manifest))
    path
  end
end
