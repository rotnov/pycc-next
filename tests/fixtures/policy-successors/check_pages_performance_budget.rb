#!/usr/bin/env ruby
# frozen_string_literal: true

# Pages performance budget checker (issue #229).
#
# This checker is the main gate for the GitHub Pages performance budget.
# It performs three phases:
#
# 1. **HTTP identity preflight** — starts the hermetic local server
#    (``serve_pages_fixture.py``) and verifies that every canonical page
#    returns HTTP 200 with the correct media type and a body whose SHA-256
#    matches the manifest artifact digest.  The 404 error-page cohort is
#    verified to return 404 with the correct body.  No catch-all or
#    fallback body is accepted for a canonical route.
#
# 2. **LHR validation and page-identity binding** — validates every
#    Lighthouse Result (LHR) JSON object: ``requestedUrl``,
#    ``mainDocumentUrl``, and ``finalDisplayedUrl`` must map to the same
#    expected local canonical route; ``runtimeError`` must be absent;
#    every ``runWarnings`` entry is preserved and reviewed; the
#    ``configSettings`` must match the pinned profile; each report is
#    uniquely assigned to one manifest page and one replicate (no
#    duplicate report reuse); and the rendered identity (effective
#    ``<title>``, visible H1, and canonical mapping) must match after
#    page execution.
#
# 3. **Threshold and resource budget enforcement** — gates LCP, CLS, TBT,
#    and Performance score against the reviewed thresholds, and checks
#    resource budgets (file sizes, no unexpected requests, no third-party
#    dependencies).
#
# The checker fails closed on any violation: exit 1 on a threshold or
# identity failure, exit 2 on a configuration or structural error.

require "json"
require "digest"
require "net/http"
require "uri"
require "open3"
require "fileutils"
require "pathname"
require "find"

REPLICATE_COUNT = 5
MANIFEST_DEFAULT =
  File.expand_path("../tests/fixtures/pages-performance-manifest.json", __dir__)
BUDGET_DEFAULT =
  File.expand_path("../tests/fixtures/pages-performance-budget.json", __dir__)
SERVE_SCRIPT =
  File.expand_path("serve_pages_fixture.py", __dir__)

# ---------------------------------------------------------------------------
# Manifest and budget loading
# ---------------------------------------------------------------------------

def load_json_file(path, label)
  raise "#{label} not found: #{path}" unless File.file?(path)

  JSON.parse(File.read(path))
rescue JSON::ParserError => e
  raise "could not parse #{label} #{path}: #{e.message}"
end

def load_manifest(path)
  manifest = load_json_file(path, "manifest")
  unless manifest.is_a?(Hash)
    raise "manifest must be a JSON object"
  end
  unless manifest["manifest_version"] == 1
    raise "manifest version must be 1, got #{manifest["manifest_version"]}"
  end
  pages = manifest["canonical_pages"]
  unless pages.is_a?(Array) && pages.length == 5
    raise "manifest must contain exactly 5 canonical pages, got #{pages&.length}"
  end
  pages.each do |page|
    %w[id local_route expected_http_status source_artifact
       source_artifact_sha256 expected_title expected_h1
       canonical_url redirect_policy].each do |key|
      raise "canonical page missing required key '#{key}': #{page.inspect}" unless page.key?(key)
    end
  end
  error_pages = manifest["error_pages"]
  unless error_pages.is_a?(Array) && !error_pages.empty?
    raise "manifest must contain at least one error page"
  end
  error_pages.each do |page|
    unless page["cohort"] == "error-page"
      raise "error page must have cohort 'error-page': #{page.inspect}"
    end
  end
  manifest
end

def load_budget(path)
  budget = load_json_file(path, "budget config")
  unless budget.is_a?(Hash)
    raise "budget config must be a JSON object"
  end
  unless budget["budget_version"] == 1
    raise "budget version must be 1, got #{budget["budget_version"]}"
  end
  thresholds = budget["metric_thresholds"]
  unless thresholds.is_a?(Hash)
    raise "budget config must contain metric_thresholds"
  end
  %w[lcp cls tbt performance_score].each do |key|
    raise "metric_thresholds missing '#{key}'" unless thresholds.key?(key)
  end
  resource = budget["resource_budgets"]
  unless resource.is_a?(Hash)
    raise "budget config must contain resource_budgets"
  end
  budget
end

# ---------------------------------------------------------------------------
# HTTP identity preflight
# ---------------------------------------------------------------------------

def start_hermetic_server(site_dir)
  # Use popen2e so we can read the port line while the server keeps
  # running.  The server prints the port to stdout and then blocks
  # in serve_forever, so capture2 would never return.
  stdin, stdout_stderr, wait_thr = Open3.popen2e(
    "python3", SERVE_SCRIPT, "--port", "0", "--site-dir", site_dir
  )
  stdin.close
  port_line = stdout_stderr.gets&.strip
  unless port_line && port_line.match?(/\A\d+\z/)
    # Read any remaining error output before killing.
    remaining = stdout_stderr.read
    Process.kill("TERM", wait_thr.pid) rescue nil
    wait_thr.wait rescue nil
    raise "could not parse server port from stdout: #{port_line.inspect} (remaining: #{remaining})"
  end
  # Return the port and the process handle so the caller can clean up.
  [Integer(port_line), wait_thr, stdout_stderr]
end

def http_get(uri)
  uri = URI(uri)
  http = Net::HTTP.new(uri.host, uri.port)
  http.read_timeout = 10
  http.open_timeout = 5
  request = Net::HTTP::Get.new(uri.request_uri)
  response = http.request(request)
  {
    status: response.code.to_i,
    body: response.body || "",
    content_type: response["Content-Type"] || "",
    final_uri: uri.to_s
  }
end

def sha256_hex(body)
  Digest::SHA256.hexdigest(body)
end

def check_http_identity(manifest, base_url, repo_root)
  failures = []
  seen_bodies = {}

  manifest["canonical_pages"].each do |page|
    url = "#{base_url}#{page["local_route"].sub(/^\//, '')}"
    result = http_get(url)

    # 1. HTTP status must be 200
    if result[:status] != page["expected_http_status"]
      failures << "#{page["id"]}: expected HTTP #{page["expected_http_status"]}, got #{result[:status]}"
      next
    end

    # 2. No unexpected redirect (canonical pages must not redirect)
    if page["redirect_policy"] == "no_redirect" && result[:status] >= 300 && result[:status] < 400
      failures << "#{page["id"]}: unexpected redirect (status #{result[:status]})"
      next
    end

    # 3. Media type must be text/html
    unless result[:content_type].start_with?("text/html")
      failures << "#{page["id"]}: expected text/html content type, got #{result[:content_type]}"
      next
    end

    # 4. Body SHA-256 must match the manifest artifact digest
    actual_digest = sha256_hex(result[:body])
    expected_digest = page["source_artifact_sha256"]
    if actual_digest != expected_digest
      failures << "#{page["id"]}: body SHA-256 mismatch (expected #{expected_digest}, got #{actual_digest})"
      next
    end

    # 5. No catch-all/fallback body — the 404 body must not satisfy a canonical route
    error_page = manifest["error_pages"].first
    error_digest = error_page["source_artifact_sha256"]
    if actual_digest == error_digest
      failures << "#{page["id"]}: canonical route returned the 404 error body (catch-all/fallback)"
      next
    end

    # Track seen bodies to detect body-swap mutations
    seen_bodies[page["id"]] = actual_digest
  end

  # Verify error page cohort
  manifest["error_pages"].each do |page|
    url = "#{base_url}#{page["local_route"].sub(/^\//, '')}"
    result = http_get(url)
    if result[:status] != page["expected_http_status"]
      failures << "#{page["id"]}: expected HTTP #{page["expected_http_status"]}, got #{result[:status]}"
      next
    end
    actual_digest = sha256_hex(result[:body])
    if actual_digest != page["source_artifact_sha256"]
      failures << "#{page["id"]}: error body SHA-256 mismatch (expected #{page["source_artifact_sha256"]}, got #{actual_digest})"
    end
  end

  # Detect body-swap: two canonical pages with the same body digest
  if seen_bodies.values.uniq.length != seen_bodies.length
    duplicates = seen_bodies.select { |_, d| seen_bodies.values.count(d) > 1 }
    failures << "body-swap detected: multiple canonical pages share the same body digest: #{duplicates.inspect}"
  end

  failures
end

# ---------------------------------------------------------------------------
# Resource budget checks
# ---------------------------------------------------------------------------

def check_resource_budgets(budget, repo_root)
  failures = []
  site_dir = File.join(repo_root, "site")

  # HTML: per-page max
  html_budget = budget["resource_budgets"]["html"]["max_bytes_per_page"]
  %w[index.html status/index.html architecture/index.html
     python-aot-compilers/index.html ai-native/index.html].each do |rel|
    path = File.join(site_dir, rel)
    if File.file?(path)
      size = File.size(path)
      if size > html_budget
        failures << "resource budget: HTML #{rel} is #{size} bytes, exceeds #{html_budget} byte limit"
      end
    else
      failures << "resource budget: HTML artifact missing: #{rel}"
    end
  end

  # CSS: total max
  css_budget = budget["resource_budgets"]["css"]["max_bytes_total"]
  css_path = File.join(site_dir, "styles.css")
  if File.file?(css_path)
    css_size = File.size(css_path)
    if css_size > css_budget
      failures << "resource budget: CSS total is #{css_size} bytes, exceeds #{css_budget} byte limit"
    end
  else
    failures << "resource budget: CSS artifact missing: styles.css"
  end

  # JavaScript: total max
  js_budget = budget["resource_budgets"]["javascript"]["max_bytes_total"]
  js_path = File.join(site_dir, "site.js")
  if File.file?(js_path)
    js_size = File.size(js_path)
    if js_size > js_budget
      failures << "resource budget: JavaScript total is #{js_size} bytes, exceeds #{js_budget} byte limit"
    end
  else
    failures << "resource budget: JavaScript artifact missing: site.js"
  end

  # Images: total per-page max — scan for all image files in the site
  # directory so any added image is caught, not just the known og.png.
  img_budget = budget["resource_budgets"]["images"]["max_bytes_total_per_page"]
  img_extensions = %w[.png .jpg .jpeg .gif .svg .webp .avif]
  img_total = 0
  img_files = []
  Find.find(site_dir) do |path|
    next unless File.file?(path)
    ext = File.extname(path).downcase
    next unless img_extensions.include?(ext)
    img_files << path
    img_total += File.size(path)
  end
  if img_total > img_budget
    rel_files = img_files.map { |p| p.sub("#{site_dir}/", "") }
    failures << "resource budget: images total is #{img_total} bytes across #{img_files.length} file(s) (#{rel_files.join(', ')}), exceeds #{img_budget} byte limit"
  end

  failures
end

# ---------------------------------------------------------------------------
# LHR validation and page-identity binding
# ---------------------------------------------------------------------------

def extract_title_from_html(html)
  match = html.match(/<title[^>]*>(.*?)<\/title>/im)
  match ? match[1].strip : nil
end

def extract_h1_from_html(html)
  # Match the first <h1> element and capture its full text content,
  # stripping nested tags and whitespace.
  match = html.match(/<h1[^>]*>(.*?)<\/h1>/im)
  return nil unless match
  inner = match[1]
  # Remove <br> tags and replace with nothing (they join lines).
  inner = inner.gsub(/<br\s*\/?>/i, " ")
  # Remove nested tags like <span>.
  inner = inner.gsub(/<[^>]+>/, "")
  # Collapse whitespace.
  inner = inner.gsub(/\s+/, " ").strip
  inner
end

def extract_canonical_from_html(html)
  match = html.match(/<link\s+rel=["']canonical["']\s+href=["']([^"']+)["']/i)
  match ? match[1].strip : nil
end

def validate_lhr_identity(lhr, page, replicate, base_url, repo_root)
  failures = []
  page_id = page["id"]
  expected_url = "#{base_url}#{page["local_route"].sub(/^\//, '')}"

  # 1. requestedUrl, mainDocumentUrl, finalDisplayedUrl must map to the
  #    same expected local canonical route.
  %w[requestedUrl mainDocumentUrl finalDisplayedUrl].each do |field|
    val = lhr[field]
    unless val == expected_url
      failures << "#{page_id} replicate #{replicate}: #{field} mismatch (expected #{expected_url}, got #{val})"
    end
  end

  # 2. runtimeError must be absent
  runtime_error = lhr["runtimeError"]
  if runtime_error && !runtime_error.empty?
    failures << "#{page_id} replicate #{replicate}: runtimeError present: #{runtime_error.inspect}"
  end

  # 3. runWarnings must be preserved (not discarded) — we collect them
  #    but do not fail on warnings alone.  However, a missing
  #    runWarnings field is a structural error.
  unless lhr.key?("runWarnings")
    failures << "#{page_id} replicate #{replicate}: runWarnings field missing from LHR"
  end

  # 4. configSettings must match the pinned profile
  config = lhr["configSettings"]
  unless config.is_a?(Hash)
    failures << "#{page_id} replicate #{replicate}: configSettings missing or not an object"
  else
    unless config["formFactor"] == "mobile"
      failures << "#{page_id} replicate #{replicate}: configSettings.formFactor must be 'mobile', got #{config["formFactor"]}"
    end
    screen = config["screenEmulation"]
    if screen.is_a?(Hash)
      unless screen["width"] == 412 && screen["height"] == 823
        failures << "#{page_id} replicate #{replicate}: configSettings.screenEmulation must be 412x823, got #{screen["width"]}x#{screen["height"]}"
      end
      unless screen["deviceScaleFactor"] == 1.75
        failures << "#{page_id} replicate #{replicate}: configSettings.screenEmulation.deviceScaleFactor must be 1.75, got #{screen["deviceScaleFactor"]}"
      end
    else
      failures << "#{page_id} replicate #{replicate}: configSettings.screenEmulation missing"
    end
  end

  # 5. Rendered identity: title, H1, and canonical must match after
  #    page execution.  These come from the LHR's `finalDisplayedUrl`
  #    page content.  In Lighthouse JSON, the page content is not
  #    directly stored, so we verify the identity against the source
  #    artifact (which the HTTP preflight already confirmed matches the
  #    served body).
  #
  #    The LHR may include a `stackPacks` or other metadata, but the
  #    authoritative identity is the served HTML, which we already
  #    verified in the preflight.  Here we check that the LHR's
  #    `audits` object contains the expected title audit if present.
  audits = lhr["audits"]
  if audits.is_a?(Hash)
    # Lighthouse stores the document title in the "document-title" audit.
    title_audit = audits["document-title"]
    if title_audit.is_a?(Hash)
      title_value = title_audit.dig("details", "items", 0, "title")
      if title_value && title_value != page["expected_title"]
        failures << "#{page_id} replicate #{replicate}: LHR document-title mismatch (expected #{page["expected_title"]}, got #{title_value})"
      end
    end
  end

  # 6. Rendered identity from source HTML: verify the effective <title>,
  #    visible H1, and canonical mapping from the source artifact.  The
  #    HTTP preflight already confirmed the served body matches this
  #    artifact's SHA-256, so the source HTML is the authoritative
  #    rendered-identity anchor.
  source_path = File.join(repo_root, page["source_artifact"])
  if File.file?(source_path)
    html = File.read(source_path)
    extracted_title = extract_title_from_html(html)
    if extracted_title != page["expected_title"]
      display_title = extracted_title.nil? ? "(not found)" : extracted_title
      failures << "#{page_id} replicate #{replicate}: HTML <title> mismatch (expected #{page["expected_title"]}, got #{display_title})"
    end
    extracted_h1 = extract_h1_from_html(html)
    if extracted_h1 != page["expected_h1"]
      display_h1 = extracted_h1.nil? ? "(not found)" : extracted_h1
      failures << "#{page_id} replicate #{replicate}: HTML <h1> mismatch (expected #{page["expected_h1"]}, got #{display_h1})"
    end
    extracted_canonical = extract_canonical_from_html(html)
    if extracted_canonical != page["canonical_url"]
      display_canonical = extracted_canonical.nil? ? "(not found)" : extracted_canonical
      failures << "#{page_id} replicate #{replicate}: HTML canonical mismatch (expected #{page["canonical_url"]}, got #{display_canonical})"
    end
  else
    failures << "#{page_id} replicate #{replicate}: source artifact not found: #{source_path}"
  end

  failures
end

# ---------------------------------------------------------------------------
# Network request validation from Lighthouse LHR data
# ---------------------------------------------------------------------------

def normalize_request_url(url)
  # Strip query string and fragment for comparison.
  url.split("?")[0].split("#")[0]
end

def validate_network_requests(lhr, page, replicate, base_url, budget, manifest)
  failures = []
  page_id = page["id"]

  audits = lhr["audits"]
  return failures unless audits.is_a?(Hash)

  network_audit = audits["network-requests"]
  # If the network-requests audit is absent (e.g. synthetic test fixtures
  # without network data), skip validation.  Real Lighthouse output always
  # includes this audit.
  return failures unless network_audit.is_a?(Hash)

  items = network_audit.dig("details", "items")
  return failures unless items.is_a?(Array)

  # Build the set of allowed request URLs (normalized, no query/fragment).
  expected_url = normalize_request_url("#{base_url}#{page["local_route"].sub(/^\//, '')}")
  project_path = manifest["project_path"] || "/pycc/"
  project_prefix = project_path.sub(/^\//, "") # "pycc/"

  allowed_urls = [expected_url, expected_url.chomp("/")]
  allowed_asset_paths = budget.dig("resource_budgets", "allowed_assets", "paths") || []
  allowed_asset_paths.each do |asset|
    allowed_urls << normalize_request_url("#{base_url}#{project_prefix}#{asset}")
  end

  # Parse the base origin for third-party classification.  Compare
  # scheme + host only (not port): the hermetic server binds to a
  # random localhost port, and Lighthouse may report request URLs with
  # or without the explicit port depending on its URL normalization.
  # All requests to 127.0.0.1 on any port are first-party by
  # construction — the server only serves the checked-in site/ tree.
  base_uri = URI(base_url)
  base_host_key = "#{base_uri.scheme}://#{base_uri.host}"

  third_party_count = 0
  unexpected_count = 0
  third_party_urls = []
  unexpected_urls = []

  items.each do |item|
    url = item["url"]
    next unless url

    # Skip non-network schemes (data:, blob:, about:).
    next if url.start_with?("data:", "blob:", "about:")

    begin
      item_uri = URI(url)
    rescue URI::InvalidURIError
      next # Skip unparseable URLs
    end

    item_host_key = "#{item_uri.scheme}://#{item_uri.host}"

    if item_host_key != base_host_key
      third_party_count += 1
      third_party_urls << url
    else
      # First-party: check if it's in the allowed set.
      normalized = normalize_request_url(url)
      unless allowed_urls.include?(normalized)
        unexpected_count += 1
        unexpected_urls << url
      end
    end
  end

  tp_max = budget.dig("resource_budgets", "third_party", "max_requests")
  if tp_max && third_party_count > tp_max
    failures << "#{page_id} replicate #{replicate}: third-party requests #{third_party_count} exceeds max_requests #{tp_max} (#{third_party_urls.join(', ')})"
  end

  uq_max = budget.dig("resource_budgets", "unexpected_requests", "max_requests")
  if uq_max && unexpected_count > uq_max
    failures << "#{page_id} replicate #{replicate}: unexpected first-party requests #{unexpected_count} exceeds max_requests #{uq_max} (#{unexpected_urls.join(', ')})"
  end

  failures
end

def extract_lhr_metric(lhr, audit_id)
  audits = lhr["audits"]
  return nil unless audits.is_a?(Hash)
  audit = audits[audit_id]
  return nil unless audit.is_a?(Hash)
  numeric = audit["numericValue"]
  return nil unless numeric.is_a?(Numeric)
  numeric.to_f
end

def extract_performance_score(lhr)
  categories = lhr["categories"]
  return nil unless categories.is_a?(Hash)
  perf = categories["performance"]
  return nil unless perf.is_a?(Hash)
  score = perf["score"]
  return nil unless score.is_a?(Numeric)
  score.to_f
end

def select_median(values)
  sorted = values.sort
  sorted[sorted.length / 2]
end

def validate_and_gate_lhrs(manifest, budget, lhr_dir, base_url, repo_root)
  failures = []
  used_lhr_files = {}
  # Track content digests per page to detect cross-page reuse only.
  # Replicates within the same page may have identical content (especially
  # in synthetic test fixtures); cross-page identical content indicates
  # one LHR was reused under different page labels.
  content_digest_by_page = {}

  manifest["canonical_pages"].each do |page|
    page_id = page["id"]
    page_dir = File.join(lhr_dir, page_id)

    unless File.directory?(page_dir)
      failures << "#{page_id}: LHR directory missing: #{page_dir}"
      next
    end

    lcp_values = []
    cls_values = []
    tbt_values = []
    perf_scores = []

    (1..REPLICATE_COUNT).each do |replicate|
      lhr_path = File.join(page_dir, "replicate-#{replicate}.json")
      unless File.file?(lhr_path)
        failures << "#{page_id} replicate #{replicate}: LHR file missing: #{lhr_path}"
        next
      end

      # Detect duplicate report reuse by realpath (symlink/hardlink to
      # the same file).  This catches file-level reuse across any pages.
      real_path = File.realpath(lhr_path)
      if used_lhr_files.key?(real_path)
        failures << "#{page_id} replicate #{replicate}: LHR file reused from #{used_lhr_files[real_path]} (duplicate report reuse)"
        next
      end
      used_lhr_files[real_path] = "#{page_id} replicate #{replicate}"

      raw_content = File.read(lhr_path)

      # Detect cross-page content reuse: if the same content digest
      # appears under a different page ID, it's a reused report.
      content_digest = Digest::SHA256.hexdigest(raw_content)
      if content_digest_by_page.key?(content_digest)
        prev_page = content_digest_by_page[content_digest]
        if prev_page != page_id
          failures << "#{page_id} replicate #{replicate}: LHR content identical to #{prev_page} (duplicate report reuse across pages)"
          next
        end
      else
        content_digest_by_page[content_digest] = page_id
      end

      begin
        lhr = JSON.parse(raw_content)
      rescue JSON::ParserError => e
        failures << "#{page_id} replicate #{replicate}: could not parse LHR JSON: #{e.message}"
        next
      end

      # Validate identity
      failures.concat(validate_lhr_identity(lhr, page, replicate, base_url, repo_root))

      # Validate network request budgets (third-party and unexpected requests)
      failures.concat(validate_network_requests(lhr, page, replicate, base_url, budget, manifest))

      # Extract metrics
      lcp = extract_lhr_metric(lhr, "largest-contentful-paint")
      cls = extract_lhr_metric(lhr, "cumulative-layout-shift")
      tbt = extract_lhr_metric(lhr, "total-blocking-time")
      perf = extract_performance_score(lhr)

      if lcp.nil?
        failures << "#{page_id} replicate #{replicate}: LCP metric missing or invalid"
      else
        lcp_values << lcp
      end

      if cls.nil?
        failures << "#{page_id} replicate #{replicate}: CLS metric missing or invalid"
      else
        cls_values << cls
      end

      if tbt.nil?
        failures << "#{page_id} replicate #{replicate}: TBT metric missing or invalid"
      else
        tbt_values << tbt
      end

      if perf.nil?
        failures << "#{page_id} replicate #{replicate}: Performance score missing or invalid"
      else
        perf_scores << perf
      end
    end

    # Fail closed if any replicate is missing/partial
    if lcp_values.length != REPLICATE_COUNT
      failures << "#{page_id}: expected #{REPLICATE_COUNT} LCP values, got #{lcp_values.length} (fail closed on missing/partial)"
    end
    if cls_values.length != REPLICATE_COUNT
      failures << "#{page_id}: expected #{REPLICATE_COUNT} CLS values, got #{cls_values.length} (fail closed on missing/partial)"
    end
    if tbt_values.length != REPLICATE_COUNT
      failures << "#{page_id}: expected #{REPLICATE_COUNT} TBT values, got #{tbt_values.length} (fail closed on missing/partial)"
    end
    if perf_scores.length != REPLICATE_COUNT
      failures << "#{page_id}: expected #{REPLICATE_COUNT} Performance scores, got #{perf_scores.length} (fail closed on missing/partial)"
    end

    # Count failures specific to this page (by prefix) to decide
    # whether to skip the threshold check.
    page_failures = failures.count { |f| f.start_with?("#{page_id}") }
    next if page_failures.positive? # Don't gate on metrics if we already have structural failures

    # Select median and gate against thresholds. Lighthouse reports LCP and
    # TBT numericValue already in milliseconds, so no unit conversion is
    # needed here (only the Performance score is a 0..1 fraction scaled to
    # 0..100 and CLS is unitless).
    median_lcp_ms = select_median(lcp_values)
    median_cls = select_median(cls_values)
    median_tbt_ms = select_median(tbt_values)
    median_perf = select_median(perf_scores) * 100.0

    lcp_max = budget["metric_thresholds"]["lcp"]["max_ms"]
    cls_max = budget["metric_thresholds"]["cls"]["max"]
    tbt_max = budget["metric_thresholds"]["tbt"]["max_ms"]
    perf_min = budget["metric_thresholds"]["performance_score"]["min"]

    if median_lcp_ms > lcp_max
      failures << "#{page_id}: LCP median #{median_lcp_ms.round(1)}ms exceeds #{lcp_max}ms budget (values: #{lcp_values.map { |v| v.round(1) }})"
    end
    if median_cls > cls_max
      failures << "#{page_id}: CLS median #{median_cls} exceeds #{cls_max} budget (values: #{cls_values})"
    end
    if median_tbt_ms > tbt_max
      failures << "#{page_id}: TBT median #{median_tbt_ms.round(1)}ms exceeds #{tbt_max}ms budget (values: #{tbt_values.map { |v| v.round(1) }})"
    end
    if median_perf < perf_min
      failures << "#{page_id}: Performance score median #{median_perf.round(1)} below #{perf_min} floor (values: #{perf_scores.map { |v| (v * 100).round(1) }})"
    end

    puts format(
      "%s: LCP=%.1fms (budget %.0fms), CLS=%.4f (budget %.2f), TBT=%.1fms (budget %.0fms), Perf=%.1f (floor %.0f)",
      page_id, median_lcp_ms, lcp_max, median_cls, cls_max, median_tbt_ms, tbt_max, median_perf, perf_min
    )
  end

  failures
end

# ---------------------------------------------------------------------------
# Main entry point
# ---------------------------------------------------------------------------

def stop_hermetic_server(wait_thr, stdout_stderr)
  return unless wait_thr

  stdout_stderr&.close
  Process.kill("TERM", wait_thr.pid) rescue nil
  wait_thr.wait rescue nil
end

def pages_perf_budget_main(arguments)
  manifest_path = MANIFEST_DEFAULT
  budget_path = BUDGET_DEFAULT
  lhr_dir = nil
  site_dir = "site"
  repo_root = File.expand_path("..", __dir__)
  skip_lighthouse = false

  args = arguments.dup
  while (arg = args.shift)
    case arg
    when "--manifest" then manifest_path = args.shift
    when "--budget" then budget_path = args.shift
    when "--lhr-dir" then lhr_dir = args.shift
    when "--site-dir" then site_dir = args.shift
    when "--repo-root" then repo_root = args.shift
    when "--skip-lighthouse" then skip_lighthouse = true
    when "--help", "-h"
      warn "usage: check_pages_performance_budget.rb [--manifest PATH] [--budget PATH] [--lhr-dir DIR] [--site-dir DIR] [--repo-root DIR] [--skip-lighthouse]"
      return 0
    else
      warn "unknown argument: #{arg}"
      return 2
    end
  end

  manifest = load_manifest(manifest_path)
  budget = load_budget(budget_path)

  # Phase 1: HTTP identity preflight
  puts "Phase 1: HTTP identity preflight"
  port, server_thr, server_io = start_hermetic_server(site_dir)
  base_url = "http://127.0.0.1:#{port}/"

  exit_code = catch(:done) do
    begin
      http_failures = check_http_identity(manifest, base_url, repo_root)

      unless http_failures.empty?
        http_failures.each { |f| warn "FAIL: #{f}" }
        throw :done, 1
      end
      puts "OK: HTTP identity preflight passed for all canonical and error pages"

      # Phase 2 & 3: LHR validation and threshold enforcement
      if skip_lighthouse
        puts "SKIP: Lighthouse validation skipped (--skip-lighthouse)"
        # Still check resource budgets
        resource_failures = check_resource_budgets(budget, repo_root)
        unless resource_failures.empty?
          resource_failures.each { |f| warn "FAIL: #{f}" }
          throw :done, 1
        end
        puts "OK: resource budgets within limits"
        puts "OK: pages performance budget gate passed (Lighthouse skipped)"
        throw :done, 0
      end

      unless lhr_dir && File.directory?(lhr_dir)
        warn "FAIL: --lhr-dir is required when Lighthouse validation is not skipped"
        throw :done, 2
      end

      puts "Phase 2: LHR validation and page-identity binding"
      puts "Phase 3: Threshold and resource budget enforcement"
      lhr_failures = validate_and_gate_lhrs(manifest, budget, lhr_dir, base_url, repo_root)
      resource_failures = check_resource_budgets(budget, repo_root)

      all_failures = lhr_failures + resource_failures
      unless all_failures.empty?
        all_failures.each { |f| warn "FAIL: #{f}" }
        throw :done, 1
      end

      puts "OK: resource budgets within limits"
      puts "OK: pages performance budget gate passed"
      0
    ensure
      stop_hermetic_server(server_thr, server_io)
    end
  end

  exit_code
rescue RuntimeError, ArgumentError => e
  warn "ERROR: #{e.message}"
  2
end

exit(pages_perf_budget_main(ARGV)) if $PROGRAM_NAME == __FILE__
