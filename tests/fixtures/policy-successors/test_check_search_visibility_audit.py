#!/usr/bin/env python3
"""Mutation tests for the base-owned search visibility audit."""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path
import tempfile
import unittest

AUDIT_PATH = Path(__file__).with_name("check_search_visibility_audit.py")
AUDIT_SPEC = importlib.util.spec_from_file_location(
    "trusted_search_visibility_audit", AUDIT_PATH
)
if AUDIT_SPEC is None or AUDIT_SPEC.loader is None:
    raise RuntimeError("could not load the trusted search visibility auditor")
AUDIT_MODULE = importlib.util.module_from_spec(AUDIT_SPEC)
AUDIT_SPEC.loader.exec_module(AUDIT_MODULE)
ACTIVATED_AT = AUDIT_MODULE.ACTIVATED_AT
AuditError = AUDIT_MODULE.AuditError
GITHUB_SURFACE_CONTRACT = AUDIT_MODULE.GITHUB_SURFACE_CONTRACT
GITHUB_SURFACE = AUDIT_MODULE.GITHUB_SURFACE
GOOGLE_SURFACE_CONTRACT = AUDIT_MODULE.GOOGLE_SURFACE_CONTRACT
INTERPRETATION_PROJECTION_HEADER = AUDIT_MODULE.INTERPRETATION_PROJECTION_HEADER
history_digest = AUDIT_MODULE.history_digest
history_rows = AUDIT_MODULE.history_rows
latest_history_projection_lines = AUDIT_MODULE.latest_history_projection_lines
semantic_identity = AUDIT_MODULE.semantic_identity
validate = AUDIT_MODULE.validate


class SearchVisibilityAuditTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="pycc-search-audit-")
        root = Path(self.temporary.name)
        self.base = root / "base"
        self.head = root / "head"
        (self.base / "docs").mkdir(parents=True)
        (self.head / "docs").mkdir(parents=True)

        base_row = (
            "| 2026-07-29T00:00:00Z | `python aot compiler` | 19 | — | 28 | 28 |"
        )
        head_row = (
            "| 2026-07-31T00:00:00Z | `python aot compiler` | 7 | +12 | 50 | 240 |"
        )
        self.base_visibility = self.visibility(base_row)
        self.head_visibility = self.visibility(base_row, head_row)
        (self.base / "docs" / "SEARCH_VISIBILITY.md").write_text(
            self.base_visibility
        )
        (self.head / "docs" / "SEARCH_VISIBILITY.md").write_text(
            self.head_visibility
        )
        self.registry = {
            "registry_version": 1,
            "registry_activated_at": ACTIVATED_AT,
            "semantic_identity_versions": {
                "github_repository_search": "github-repository-search-v1",
                "google_web": "google-web-query-v1",
            },
            "surfaces": {
                "github_repository_search": GITHUB_SURFACE_CONTRACT,
                "google_web": GOOGLE_SURFACE_CONTRACT,
            },
            "queries": [
                {
                    "id": "github-product-python-aot-compiler",
                    "surface": "github_repository_search",
                    "raw_query": "python aot compiler",
                    "semantic_identity": (
                        "github-repository-search-v1:bag:aot compiler python"
                    ),
                    "semantic_identity_version": "github-repository-search-v1",
                    "intent_class": "product_category",
                    "lifecycle": "active",
                    "kpi_role": "product_acquisition",
                    "rationale": "Synthetic product query for audit mutations.",
                    "activated_at": ACTIVATED_AT,
                    "retired_at": None,
                    "alias_of": None,
                },
                {
                    "id": "github-authorship-ai-native-compiler",
                    "surface": "github_repository_search",
                    "raw_query": "AI-native compiler",
                    "semantic_identity": (
                        "github-repository-search-v1:syntax:ai-native compiler"
                    ),
                    "semantic_identity_version": "github-repository-search-v1",
                    "intent_class": "authorship_narrative",
                    "lifecycle": "retired",
                    "kpi_role": "excluded",
                    "rationale": "Synthetic retired authorship query.",
                    "activated_at": "2026-07-24T23:02:03Z",
                    "retired_at": "2026-07-29T10:25:27Z",
                    "alias_of": None,
                },
            ],
            "measurements": [
                {
                    "snapshot_id": "github-2026-07-31-python-aot-compiler",
                    "observed_at": "2026-07-31T00:00:00Z",
                    "query_id": "github-product-python-aot-compiler",
                    "surface": "github_repository_search",
                    "provider": "github",
                    "request_parameters": {
                        "q": "python aot compiler",
                        "per_page": 50,
                    },
                    "sort_contract": "default_best_match",
                    "result_window": 50,
                    "returned_results": 50,
                    "api_total": 240,
                    "target_rank": 7,
                    "incomplete_results": False,
                    "ordered_corpus_sha256": "a" * 64,
                }
            ],
        }
        base_registry = {**self.registry, "measurements": []}
        (self.base / "docs" / "SEARCH_QUERY_REGISTRY.json").write_text(
            json.dumps(base_registry, indent=2) + "\n"
        )
        self.write_registry()
        self.refresh_checkpoint()
        self.audited_at = datetime(2026, 8, 1, tzinfo=timezone.utc)

    def tearDown(self) -> None:
        self.temporary.cleanup()

    @staticmethod
    def visibility(*rows: str) -> str:
        history = (
            "# Search Visibility Measurements\n\n"
            "## GitHub repository search history\n\n"
            "| Observed at (UTC) | Exact query | Rank | Δ | Results | Total |\n"
            "|---|---|---:|---:|---:|---:|\n"
            + "\n".join(rows)
            + "\n\n## GitHub traffic history\n"
        )
        projection = "\n".join(latest_history_projection_lines(history_rows(history)))
        return (
            history
            + "\n## Current interpretation\n\n"
            + projection
            + "\n\nSynthetic interpretation for mutation tests.\n"
        )

    def write_registry(self) -> None:
        (self.head / "docs" / "SEARCH_QUERY_REGISTRY.json").write_text(
            json.dumps(self.registry, indent=2) + "\n"
        )

    def refresh_checkpoint(self) -> None:
        rows = history_rows(
            (self.head / "docs" / "SEARCH_VISIBILITY.md").read_text()
        )
        checkpoint = {
            "required_prefix_rows": len(rows),
            "sha256": history_digest(rows),
        }
        document = {
            "checkpoint_version": 1,
            "surfaces": {"github_repository_search": [checkpoint]},
        }
        (
            self.head / "docs" / "SEARCH_VISIBILITY_CHECKPOINTS.json"
        ).write_text(json.dumps(document, indent=2) + "\n")
        marker = (
            "<!-- search-history-checkpoint: github_repository_search "
            f"{checkpoint['required_prefix_rows']} {checkpoint['sha256']} -->"
        )
        (self.head / "docs" / "ROADMAP.md").write_text(
            f"# Roadmap\n\n{marker}\n"
        )

    def install_reviewed_bootstrap(self) -> list[list[str]]:
        repository_root = Path(__file__).resolve().parents[1]
        visibility = (repository_root / "docs" / "SEARCH_VISIBILITY.md").read_text()
        rows = history_rows(visibility)
        self.base.joinpath("docs", "SEARCH_VISIBILITY.md").write_text(
            self.visibility(
                *("| " + " | ".join(row) + " |" for row in rows[:108])
            )
        )
        self.base.joinpath("docs", "SEARCH_QUERY_REGISTRY.json").unlink()
        for name in (
            "SEARCH_VISIBILITY.md",
            "SEARCH_QUERY_REGISTRY.json",
            "SEARCH_VISIBILITY_CHECKPOINTS.json",
        ):
            self.head.joinpath("docs", name).write_text(
                repository_root.joinpath("docs", name).read_text()
            )
        checkpoint_document = json.loads(
            self.head.joinpath(
                "docs", "SEARCH_VISIBILITY_CHECKPOINTS.json"
            ).read_text()
        )
        markers = [
            "<!-- search-history-checkpoint: github_repository_search "
            f"{checkpoint['required_prefix_rows']} {checkpoint['sha256']} -->"
            for checkpoint in checkpoint_document["surfaces"][
                "github_repository_search"
            ]
        ]
        self.head.joinpath("docs", "ROADMAP.md").write_text(
            "# Roadmap\n\n" + "\n".join(markers) + "\n"
        )
        return rows

    def test_valid_append_passes(self) -> None:
        validate(self.head, self.base, self.audited_at)

    def test_current_interpretation_projection_tracks_latest_rows(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        path.write_text(
            path.read_text().replace(
                "| 2026-07-31T00:00:00Z | `python aot compiler` | 7 | +12 | 50 | 240 |",
                "| 2026-07-31T00:00:00Z | `python aot compiler` | 6 | +13 | 50 | 240 |",
                1,
            )
        )
        self.registry["measurements"][0]["target_rank"] = 6
        self.write_registry()
        self.refresh_checkpoint()
        with self.assertRaisesRegex(AuditError, "latest-search projection"):
            validate(self.head, self.base, self.audited_at)

    def test_current_interpretation_projection_requires_block_boundary(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        path.write_text(
            path.read_text().replace(
                "| `python aot compiler` | 2026-07-31T00:00:00Z | 7 | 50 | 240 |\n\nSynthetic",
                "| `python aot compiler` | 2026-07-31T00:00:00Z | 7 | 50 | 240 |\nSynthetic",
                1,
            )
        )
        with self.assertRaisesRegex(AuditError, "block boundary"):
            validate(self.head, self.base, self.audited_at)

    def test_current_interpretation_projection_cannot_be_duplicated(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        path.write_text(path.read_text() + f"\n{INTERPRETATION_PROJECTION_HEADER}\n")
        with self.assertRaisesRegex(AuditError, "exactly one latest-search projection"):
            validate(self.head, self.base, self.audited_at)

    def test_reviewed_bootstrap_passes_without_a_base_registry(self) -> None:
        self.install_reviewed_bootstrap()
        validate(self.head, self.base, self.audited_at)

    def test_initial_registry_rejects_an_unreviewed_bootstrap(self) -> None:
        rows = self.install_reviewed_bootstrap()
        checkpoint_path = (
            self.head / "docs" / "SEARCH_VISIBILITY_CHECKPOINTS.json"
        )
        document = json.loads(checkpoint_path.read_text())
        first = document["surfaces"]["github_repository_search"][0]
        first["required_prefix_rows"] = 107
        first["sha256"] = history_digest(rows[:107])
        checkpoint_path.write_text(json.dumps(document, indent=2) + "\n")
        roadmap_path = self.head / "docs" / "ROADMAP.md"
        roadmap_path.write_text(
            roadmap_path.read_text().replace(
                "github_repository_search 108 "
                "e1e44e137edce9300e75648e898b41dd3b8e25f13e06ba5264b8ee61b0fad433",
                "github_repository_search 107 " + history_digest(rows[:107]),
            )
        )
        with self.assertRaisesRegex(AuditError, "exact reviewed bootstrap"):
            validate(self.head, self.base, self.audited_at)

    def test_initial_registry_rejects_mutated_registry_bytes(self) -> None:
        self.install_reviewed_bootstrap()
        registry_path = self.head / "docs" / "SEARCH_QUERY_REGISTRY.json"
        document = json.loads(registry_path.read_text())
        document["queries"][0]["rationale"] += " Mutated after review."
        registry_path.write_text(json.dumps(document, indent=2) + "\n")
        with self.assertRaisesRegex(AuditError, "exact reviewed bootstrap"):
            validate(self.head, self.base, self.audited_at)

    def test_rewriting_trusted_history_fails(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        path.write_text(path.read_text().replace("| 19 | — |", "| 18 | — |", 1))
        self.refresh_checkpoint()
        with self.assertRaisesRegex(AuditError, "trusted base prefix"):
            validate(self.head, self.base, self.audited_at)

    def test_markdown_equivalent_duplicate_history_heading_is_rejected(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        forged_table = (
            "\n\n| Observed at (UTC) | Exact query | Rank | Δ | Results | Total |\n"
            "|---|---|---:|---:|---:|---:|\n"
            "| 2026-07-31T02:00:00Z | `forged` | 1 | — | 1 | 1 |\n\n"
        )
        for duplicate_heading in (
            "## GitHub repository search history   ",
            "## GitHub repository search history ##",
            "> ## GitHub repository search history",
            "- ## GitHub repository search history",
            "## GitHub repository search&#32;history",
            "## **GitHub repository search history**",
            "## GitHub repository search hist**or**y",
            "## GitHub repository search hist`or`y",
        ):
            with self.subTest(duplicate_heading=duplicate_heading):
                path.write_text(
                    f"{duplicate_heading}{forged_table}{self.head_visibility}"
                )
                with self.assertRaisesRegex(
                    AuditError,
                    "exactly one level-2|container boundaries|inline markup markers",
                ):
                    validate(self.head, self.base, self.audited_at)

    def test_history_headings_reject_literal_inline_markup_markers(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        canonical = "## GitHub repository search history"
        for replacement in (
            "## GitHub repository search hist_ory",
            "## GitHub repository search hist&#95;ory",
            "## GitHub repository search hist*ory",
            "## GitHub repository search hist~ory",
            "## GitHub repository search hist`ory",
        ):
            with self.subTest(replacement=replacement):
                path.write_text(
                    self.head_visibility.replace(canonical, replacement, 1)
                )
                with self.assertRaisesRegex(AuditError, "inline markup markers"):
                    validate(self.head, self.base, self.audited_at)

    def test_canonical_history_heading_must_be_top_level(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        canonical = "## GitHub repository search history"
        for replacement in (
            "> ## GitHub repository search history",
            "- ## GitHub repository search history",
            "> GitHub repository search history\n> ---",
            "- list item\n  ## GitHub repository search history",
            "1. list item\n   ## GitHub repository search history",
        ):
            with self.subTest(replacement=replacement):
                path.write_text(
                    self.head_visibility.replace(canonical, replacement, 1)
                )
                with self.assertRaisesRegex(
                    AuditError, "must be top-level|Setext underlines"
                ):
                    validate(self.head, self.base, self.audited_at)

    def test_canonical_history_heading_must_match_exactly(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        canonical = "## GitHub repository search history"
        for replacement in (
            "## Archived GitHub repository search history evidence",
            "## GitHub repository search history!",
            "## GitHub repository search history&#33;",
            "## GitHub repository search \\history",
            "## GitHub  repository search history",
        ):
            with self.subTest(replacement=replacement):
                path.write_text(
                    self.head_visibility.replace(canonical, replacement, 1)
                )
                with self.assertRaisesRegex(AuditError, "title must match exactly"):
                    validate(self.head, self.base, self.audited_at)

    def test_history_headings_reject_inline_links_and_html(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        for duplicate_heading in (
            "## [GitHub repository search history](https://example.invalid)",
            "## GitHub repository search hist[or](https://example.invalid)y",
            "## GitHub repository search hist<em>or</em>y",
            "## GitHub repository search hist<!-- hidden -->ory",
        ):
            with self.subTest(duplicate_heading=duplicate_heading):
                path.write_text(f"{duplicate_heading}\n\n{self.head_visibility}")
                with self.assertRaisesRegex(AuditError, "HTML"):
                    validate(self.head, self.base, self.audited_at)

    def test_inline_html_comment_cannot_hide_history_table(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        path.write_text(
            self.head_visibility.replace(
                "| Observed at (UTC) |",
                "prefix <!--\n| Observed at (UTC) |",
                1,
            )
            + "\n-->\n"
        )
        with self.assertRaisesRegex(AuditError, "HTML comments"):
            validate(self.head, self.base, self.audited_at)

    def test_history_headings_reject_invisible_unicode(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        for duplicate_heading in (
            "## GitHub repository search hist&#x200B;ory",
            "## GitHub repository search hist\u200bory",
            "## GitHub repository search hist&#x34f;ory",
            "## GitHub repository search hist\x01ory",
            "## GitHub repository search hist\x7fory",
        ):
            with self.subTest(duplicate_heading=duplicate_heading):
                path.write_text(f"{duplicate_heading}\n\n{self.head_visibility}")
                with self.assertRaisesRegex(AuditError, "invisible Unicode"):
                    validate(self.head, self.base, self.audited_at)

    def test_history_headings_reject_unicode_confusables(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        canonical = "## GitHub repository search history"
        forged_table = (
            "\n\n| Observed at (UTC) | Exact query | Rank | Δ | Results | Total |\n"
            "|---|---|---:|---:|---:|---:|\n"
            "| 2026-07-31T02:00:00Z | `forged` | 1 | — | 1 | 1 |\n"
        )
        for confusable in (
            "## GitHub repository search histоry",
            "## GіtHub repository search history",
            "## GitHub repοsitory search history",
        ):
            with self.subTest(confusable=confusable):
                path.write_text(
                    self.head_visibility.replace(
                        canonical,
                        f"{confusable}{forged_table}",
                        1,
                    )
                )
                with self.assertRaisesRegex(AuditError, "ASCII text"):
                    validate(self.head, self.base, self.audited_at)

    def test_history_heading_cannot_be_an_indented_code_block(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        canonical = "## GitHub repository search history"
        for replacement in (
            "    ## GitHub repository search history",
            "\t## GitHub repository search history",
            ">     ## GitHub repository search history",
        ):
            with self.subTest(replacement=replacement):
                path.write_text(self.head_visibility.replace(canonical, replacement, 1))
                with self.assertRaisesRegex(AuditError, "indented code blocks"):
                    validate(self.head, self.base, self.audited_at)

    def test_setext_duplicate_history_heading_is_rejected(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        forged_section = (
            "GitHub repository search history\n"
            "---\n\n"
            "| Observed at (UTC) | Exact query | Rank | Δ | Results | Total |\n"
            "|---|---|---:|---:|---:|---:|\n"
            "| 2026-07-31T02:00:00Z | `forged` | 1 | — | 1 | 1 |\n\n"
        )
        for heading, underline in (
            ("GitHub repository search history", "---"),
            ("> GitHub repository search history", "> ---"),
            ("- GitHub repository search history", "  ---"),
        ):
            with self.subTest(heading=heading):
                path.write_text(
                    forged_section.replace(
                        "GitHub repository search history\n---",
                        f"{heading}\n{underline}",
                        1,
                    )
                    + self.head_visibility
                )
                with self.assertRaisesRegex(
                    AuditError,
                    "exactly one level-2|Setext underlines",
                ):
                    validate(self.head, self.base, self.audited_at)

    def test_standalone_setext_or_thematic_underline_is_rejected(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        for underline in ("---", "===", "> ---", "- ---"):
            with self.subTest(underline=underline):
                path.write_text(self.head_visibility + "\n" + underline + "\n")
                with self.assertRaisesRegex(AuditError, "Setext underlines"):
                    validate(self.head, self.base, self.audited_at)

    def test_setext_title_stops_at_container_boundary(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        canonical = "## GitHub repository search history"
        path.write_text(
            self.head_visibility.replace(
                canonical,
                "GitHub repository search\n> history\n> ---",
                1,
            )
        )
        with self.assertRaisesRegex(AuditError, "Setext underlines"):
            validate(self.head, self.base, self.audited_at)

    def test_setext_title_preserves_lazy_blockquote_ancestry(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        forged_table = (
            "\n\n"
            "| Observed at (UTC) | Exact query | Rank | Δ | Results | Total |\n"
            "|---|---|---:|---:|---:|---:|\n"
            "| 2026-07-31T02:00:00Z | `forged` | 1 | — | 1 | 1 |\n"
        )
        for lazy_heading in (
            "> GitHub repository search\nhistory\n> ---",
            "> > GitHub repository search\nhistory\n> > ---",
            "- GitHub repository search\nhistory\n  ---",
            "> - GitHub repository search\nhistory\n>   ---",
        ):
            with self.subTest(lazy_heading=lazy_heading):
                path.write_text(
                    self.head_visibility + "\n" + lazy_heading + forged_table
                )
                with self.assertRaisesRegex(
                    AuditError,
                    "Setext underlines",
                ):
                    validate(self.head, self.base, self.audited_at)

    def test_canonical_setext_heading_rejects_multiline_rendering(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        canonical = "## GitHub repository search history"
        path.write_text(
            self.head_visibility.replace(
                canonical,
                "GitHub repository search\nhistory\n---",
                1,
            )
        )
        with self.assertRaisesRegex(AuditError, "Setext underlines"):
            validate(self.head, self.base, self.audited_at)

    def test_setext_history_heading_rejects_hard_line_breaks(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        canonical = "## GitHub repository search history"
        for replacement in (
            "GitHub repository search  \nhistory\n---",
            "GitHub repository search   \nhistory\n---",
        ):
            with self.subTest(replacement=replacement):
                path.write_text(
                    self.head_visibility.replace(canonical, replacement, 1)
                )
                with self.assertRaisesRegex(AuditError, "Setext underlines"):
                    validate(self.head, self.base, self.audited_at)

    def test_multiline_setext_duplicate_history_heading_is_rejected(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        forged_table = (
            "\n\n| Observed at (UTC) | Exact query | Rank | Δ | Results | Total |\n"
            "|---|---|---:|---:|---:|---:|\n"
            "| 2026-07-31T02:00:00Z | `forged` | 1 | — | 1 | 1 |\n\n"
        )
        for duplicate_heading in (
            "GitHub repository search\nhistory\n---",
            "> GitHub repository search\n> history\n> ---",
            "- GitHub repository search\n  history\n  ---",
        ):
            with self.subTest(duplicate_heading=duplicate_heading):
                path.write_text(
                    f"{duplicate_heading}{forged_table}{self.head_visibility}"
                )
                with self.assertRaisesRegex(
                    AuditError,
                    "Setext underlines",
                ):
                    validate(self.head, self.base, self.audited_at)

    def test_fenced_history_table_is_rejected(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        header = (
            "| Observed at (UTC) | Exact query | Rank | Δ | Results | Total |\n"
            "|---|---|---:|---:|---:|---:|\n"
        )
        table = header + self.head_visibility.split(header, 1)[1].split(
            "\n\n## GitHub traffic history", 1
        )[0]
        for opening, closing in (
            ("```markdown", "```"),
            ("> ~~~markdown", "> ~~~"),
        ):
            with self.subTest(opening=opening):
                path.write_text(
                    self.head_visibility.replace(
                        table,
                        f"{opening}\n{table}\n{closing}",
                    )
                )
                with self.assertRaisesRegex(AuditError, "fenced blocks"):
                    validate(self.head, self.base, self.audited_at)

    def test_display_math_history_container_is_rejected(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        header = (
            "| Observed at (UTC) | Exact query | Rank | Δ | Results | Total |\n"
            "|---|---|---:|---:|---:|---:|\n"
        )
        table = header + self.head_visibility.split(header, 1)[1].split(
            "\n\n## GitHub traffic history", 1
        )[0]
        path.write_text(
            self.head_visibility.replace(table, f"$$\n\n{table}\n$$")
        )
        with self.assertRaisesRegex(AuditError, "display-math blocks"):
            validate(self.head, self.base, self.audited_at)

    def test_raw_html_history_container_is_rejected(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        path.write_text(
            self.head_visibility.replace(
                "| Observed at (UTC) | Exact query | Rank | Δ | Results | Total |",
                "<div>\n"
                "| Observed at (UTC) | Exact query | Rank | Δ | Results | Total |",
                1,
            )
        )
        with self.assertRaisesRegex(AuditError, "HTML tags"):
            validate(self.head, self.base, self.audited_at)

    def test_inline_html_container_cannot_hide_history_table(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        header = "| Observed at (UTC) | Exact query | Rank | Δ | Results | Total |"
        next_section = "\n\n## GitHub traffic history"
        for opener in ("prefix <details>", 'prefix <details title="<">'):
            with self.subTest(opener=opener):
                path.write_text(
                    self.head_visibility.replace(
                        header,
                        f"{opener}\n{header}",
                        1,
                    ).replace(
                        next_section,
                        f"\nprefix </details>{next_section}",
                        1,
                    )
                )
                with self.assertRaisesRegex(AuditError, "HTML tags"):
                    validate(self.head, self.base, self.audited_at)

    def test_prefixed_raw_html_constructs_cannot_hide_history_table(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        header = "| Observed at (UTC) | Exact query | Rank | Δ | Results | Total |"
        for construct in ("<?audit?>", "<!AUDIT>", "<![CDATA[audit]]>"):
            with self.subTest(construct=construct):
                path.write_text(
                    self.head_visibility.replace(
                        header,
                        f"prefix {construct}\n\n{header}",
                        1,
                    )
                )
                with self.assertRaisesRegex(AuditError, "raw HTML constructs"):
                    validate(self.head, self.base, self.audited_at)

    def test_history_rows_must_follow_the_table_delimiter(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        header = (
            "| Observed at (UTC) | Exact query | Rank | Δ | Results | Total |\n"
            "|---|---|---:|---:|---:|---:|\n"
        )
        markdown = path.read_text().replace(header, "")
        path.write_text(
            markdown.replace(
                "\n\n## GitHub traffic history",
                f"\n{header}\n## GitHub traffic history",
            )
        )
        with self.assertRaisesRegex(AuditError, "must follow the table delimiter"):
            validate(self.head, self.base, self.audited_at)

    def test_history_header_must_be_first_section_content(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        header = "| Observed at (UTC) | Exact query | Rank | Δ | Results | Total |"
        for prefix in (
            "prose before the table\n",
            "  indented prose\n",
            "\u00a0\n",
            "\u2003\n",
            "\v\n",
            "\u2028\n",
        ):
            with self.subTest(prefix=prefix):
                path.write_text(
                    self.head_visibility.replace(header, f"{prefix}{header}", 1)
                )
                with self.assertRaisesRegex(AuditError, "valid block boundary"):
                    validate(self.head, self.base, self.audited_at)

    def test_history_rows_reject_non_commonmark_boundary_whitespace(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        header = "| Observed at (UTC) | Exact query | Rank | Δ | Results | Total |"
        for suffix in ("\u00a0", "\u2003", "\u202f"):
            with self.subTest(suffix=repr(suffix)):
                path.write_text(
                    self.head_visibility.replace(header, f"{header}{suffix}", 1)
                )
                with self.assertRaisesRegex(
                    AuditError, "leading and trailing pipe"
                ):
                    validate(self.head, self.base, self.audited_at)

    def test_history_table_cannot_resume_after_an_interruption(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        delimiter = "|---|---|---:|---:|---:|---:|"
        path.write_text(path.read_text().replace(delimiter, f"{delimiter}\ninterruption"))
        with self.assertRaisesRegex(AuditError, "cannot resume"):
            validate(self.head, self.base, self.audited_at)

    def test_higher_level_heading_ends_history_section(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        heading = "## GitHub repository search history\n\n"
        for boundary in ("# Replacement section\n\n", "Replacement section\n===\n\n"):
            with self.subTest(boundary=boundary):
                path.write_text(
                    self.head_visibility.replace(heading, heading + boundary, 1)
                )
                with self.assertRaisesRegex(
                    AuditError, "table is incomplete|Setext underlines"
                ):
                    validate(self.head, self.base, self.audited_at)

    def test_nested_headings_do_not_end_history_section(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        boundary = "\n\n## GitHub traffic history"
        forged_row = (
            "| 2026-07-31T02:00:00Z | `forged` | 1 | — | 1 | 1 |"
        )
        for nested_heading in (
            "> # Notes inside a quote",
            "- ## Notes inside a list",
            "- Notes\n  # Indented list continuation",
        ):
            with self.subTest(nested_heading=nested_heading):
                path.write_text(
                    self.head_visibility.replace(
                        boundary,
                        f"\n\n{nested_heading}\n\n{forged_row}{boundary}",
                        1,
                    )
                )
                with self.assertRaisesRegex(AuditError, "cannot resume"):
                    validate(self.head, self.base, self.audited_at)

    def test_history_rejects_a_visible_row_without_a_leading_pipe(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        visible_but_unbound = (
            "2026-07-31T01:00:00Z | `python aot compiler` | 1 | +6 | 50 | 240"
        )
        path.write_text(
            path.read_text().replace(
                "\n\n## GitHub traffic history",
                f"\n{visible_but_unbound}\n\n## GitHub traffic history",
            )
        )
        with self.assertRaisesRegex(AuditError, "unindented leading pipe"):
            validate(self.head, self.base, self.audited_at)

    def test_history_table_requires_exactly_one_boundary_pipe(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        header = "| Observed at (UTC) | Exact query | Rank | Δ | Results | Total |"
        appended_row = (
            "| 2026-07-31T00:00:00Z | `python aot compiler` | 7 | +12 | 50 | 240 |"
        )
        for original, replacement in (
            (header, f"|{header}|"),
            (appended_row, f"|{appended_row}|"),
            (appended_row, appended_row[:-1]),
        ):
            with self.subTest(replacement=replacement):
                path.write_text(
                    self.head_visibility.replace(original, replacement, 1)
                )
                with self.assertRaisesRegex(AuditError, "exactly one leading"):
                    validate(self.head, self.base, self.audited_at)

    def test_history_delimiter_requires_three_hyphens_per_cell(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        delimiter = "|---|---|---:|---:|---:|---:|"
        for short_delimiter in (
            "|-|-|-:|-:|-:|-:|",
            "|--|--|--:|--:|--:|--:|",
        ):
            with self.subTest(delimiter=short_delimiter):
                path.write_text(
                    self.head_visibility.replace(delimiter, short_delimiter, 1)
                )
                with self.assertRaisesRegex(AuditError, "at least three hyphens"):
                    validate(self.head, self.base, self.audited_at)

    def test_registry_activation_is_immutable(self) -> None:
        self.registry["registry_activated_at"] = "2026-08-01T00:00:00Z"
        self.write_registry()
        with self.assertRaisesRegex(AuditError, "activation timestamp is immutable"):
            validate(self.head, self.base, self.audited_at)

    def test_registry_rejects_duplicate_json_keys(self) -> None:
        path = self.head / "docs" / "SEARCH_QUERY_REGISTRY.json"
        text = path.read_text().replace(
            '"target_rank": 7,',
            '"target_rank": 999,\n      "target_rank": 7,',
            1,
        )
        path.write_text(text)
        with self.assertRaisesRegex(AuditError, "duplicate key 'target_rank'"):
            validate(self.head, self.base, self.audited_at)

    def test_new_query_activation_cannot_be_in_the_future(self) -> None:
        query = {
            **self.registry["queries"][0],
            "id": "github-product-future-python-compiler",
            "raw_query": "future python compiler",
            "semantic_identity": (
                "github-repository-search-v1:bag:compiler future python"
            ),
            "activated_at": "9999-12-31T23:59:59Z",
        }
        self.registry["queries"].append(query)
        self.write_registry()
        with self.assertRaisesRegex(AuditError, "activation cannot be in the future"):
            validate(self.head, self.base, self.audited_at)

    def test_query_retirement_cannot_be_in_the_future(self) -> None:
        self.registry["queries"][1]["retired_at"] = "9999-12-31T23:59:59Z"
        self.write_registry()
        with self.assertRaisesRegex(AuditError, "retirement cannot be in the future"):
            validate(self.head, self.base, self.audited_at)

    def test_existing_active_query_can_be_retired(self) -> None:
        query = self.registry["queries"][0]
        query["lifecycle"] = "retired"
        query["retired_at"] = "2026-07-31T00:00:01Z"
        self.write_registry()
        validate(self.head, self.base, self.audited_at)

    def test_query_retirement_must_follow_existing_measurements(self) -> None:
        query = self.registry["queries"][0]
        query["lifecycle"] = "retired"
        query["retired_at"] = "2026-07-31T00:00:00Z"
        self.write_registry()
        with self.assertRaisesRegex(AuditError, "final history observation"):
            validate(self.head, self.base, self.audited_at)

    def test_query_retirement_must_follow_legacy_history(self) -> None:
        brand_query = {
            "id": "github-brand-pycc",
            "surface": "github_repository_search",
            "raw_query": "pycc",
            "semantic_identity": "github-repository-search-v1:bag:pycc",
            "semantic_identity_version": "github-repository-search-v1",
            "intent_class": "brand",
            "lifecycle": "diagnostic",
            "kpi_role": "diagnostic",
            "rationale": "Synthetic legacy brand query.",
            "activated_at": "2026-07-24T23:02:03Z",
            "retired_at": None,
            "alias_of": None,
        }
        base_registry_path = self.base / "docs" / "SEARCH_QUERY_REGISTRY.json"
        base_registry = json.loads(base_registry_path.read_text())
        base_registry["queries"].append(brand_query)
        base_registry_path.write_text(json.dumps(base_registry, indent=2) + "\n")
        self.registry["queries"].append(
            {
                **brand_query,
                "lifecycle": "retired",
                "retired_at": "2026-07-24T23:02:03Z",
            }
        )
        self.write_registry()
        base_row = (
            "| 2026-07-29T00:00:00Z | `python aot compiler` | 19 | — | 28 | 28 |"
        )
        brand_row = "| 2026-07-30T00:00:00Z | `pycc` | >50 | — | 50 | 364 |"
        head_row = (
            "| 2026-07-31T00:00:00Z | `python aot compiler` | 7 | +12 | 50 | 240 |"
        )
        (self.base / "docs" / "SEARCH_VISIBILITY.md").write_text(
            self.visibility(base_row, brand_row)
        )
        (self.head / "docs" / "SEARCH_VISIBILITY.md").write_text(
            self.visibility(base_row, brand_row, head_row)
        )
        self.refresh_checkpoint()
        with self.assertRaisesRegex(AuditError, "final history observation"):
            validate(self.head, self.base, self.audited_at)

    def test_google_retirement_ignores_same_text_github_history(self) -> None:
        google_query = {
            "id": "google-product-python-aot-compiler",
            "surface": "google_web",
            "raw_query": "python aot compiler",
            "semantic_identity": "google-web-query-v1:raw:python aot compiler",
            "semantic_identity_version": "google-web-query-v1",
            "intent_class": "product_category",
            "lifecycle": "active",
            "kpi_role": "product_acquisition",
            "rationale": "Synthetic cross-surface query.",
            "activated_at": "2026-07-29T10:25:27Z",
            "retired_at": None,
            "alias_of": None,
        }
        base_registry_path = self.base / "docs" / "SEARCH_QUERY_REGISTRY.json"
        base_registry = json.loads(base_registry_path.read_text())
        base_registry["queries"].append(google_query)
        base_registry_path.write_text(json.dumps(base_registry, indent=2) + "\n")
        self.registry["queries"].append(
            {
                **google_query,
                "lifecycle": "retired",
                "retired_at": "2026-07-30T00:00:00Z",
            }
        )
        self.write_registry()
        validate(self.head, self.base, self.audited_at)

    def test_query_retirement_cannot_rewrite_identity(self) -> None:
        query = self.registry["queries"][0]
        query["lifecycle"] = "retired"
        query["retired_at"] = "2026-07-31T00:00:01Z"
        query["rationale"] += " Rewritten during retirement."
        self.write_registry()
        with self.assertRaisesRegex(AuditError, "trusted base queries"):
            validate(self.head, self.base, self.audited_at)

    def test_registry_raw_query_rejects_embedded_backticks(self) -> None:
        query = self.registry["queries"][0]
        query["raw_query"] = "python `aot` compiler"
        query["semantic_identity"] = (
            "github-repository-search-v1:syntax:python `aot` compiler"
        )
        self.write_registry()
        with self.assertRaisesRegex(AuditError, "cannot contain backticks"):
            validate(self.head, self.base, self.audited_at)

    def test_syntax_identity_normalizes_case_and_repeated_whitespace(self) -> None:
        version = "github-repository-search-v1"
        self.assertEqual(
            semantic_identity(
                GITHUB_SURFACE,
                "Ahead-of-time  compiler python",
                version,
            ),
            f"{version}:syntax:ahead-of-time compiler python",
        )
        common = {
            **self.registry["queries"][0],
            "semantic_identity": (
                "github-repository-search-v1:syntax:"
                "ahead-of-time compiler python"
            ),
            "activated_at": "2026-07-31T00:00:00Z",
        }
        self.registry["queries"].extend(
            [
                {
                    **common,
                    "id": "github-product-ahead-of-time-python",
                    "raw_query": "ahead-of-time compiler python",
                },
                {
                    **common,
                    "id": "github-product-ahead-of-time-python-duplicate",
                    "raw_query": "Ahead-of-time compiler python",
                },
            ]
        )
        self.write_registry()
        with self.assertRaisesRegex(AuditError, "semantic identity is double-counted"):
            validate(self.head, self.base, self.audited_at)

    def test_boolean_identity_classifies_after_case_normalization(self) -> None:
        version = "github-repository-search-v1"
        expected = f"{version}:syntax:compiler and python"
        self.assertEqual(
            semantic_identity(GITHUB_SURFACE, "compiler AND python", version),
            expected,
        )
        self.assertEqual(
            semantic_identity(GITHUB_SURFACE, "compiler and python", version),
            expected,
        )
        common = {
            **self.registry["queries"][0],
            "semantic_identity": expected,
            "activated_at": "2026-07-31T00:00:00Z",
        }
        self.registry["queries"].extend(
            [
                {
                    **common,
                    "id": "github-product-uppercase-boolean",
                    "raw_query": "compiler AND python",
                },
                {
                    **common,
                    "id": "github-product-lowercase-boolean",
                    "raw_query": "compiler and python",
                },
            ]
        )
        self.write_registry()
        with self.assertRaisesRegex(AuditError, "semantic identity is double-counted"):
            validate(self.head, self.base, self.audited_at)

    def test_github_registry_raw_query_rejects_pipes(self) -> None:
        self.registry["queries"].append(
            {
                "id": "github-diagnostic-pipe-query",
                "surface": "github_repository_search",
                "raw_query": "foo|bar",
                "semantic_identity": "github-repository-search-v1:syntax:foo|bar",
                "semantic_identity_version": "github-repository-search-v1",
                "intent_class": "brand",
                "lifecycle": "diagnostic",
                "kpi_role": "diagnostic",
                "rationale": "Synthetic unprojectable query.",
                "activated_at": "2026-07-31T00:00:00Z",
                "retired_at": None,
                "alias_of": None,
            }
        )
        self.write_registry()
        with self.assertRaisesRegex(AuditError, "cannot contain pipes"):
            validate(self.head, self.base, self.audited_at)

    def test_github_registry_raw_query_rejects_line_separators(self) -> None:
        for separator in ("\n", "\r", "\u2028"):
            with self.subTest(separator=repr(separator)):
                query = {
                    **self.registry["queries"][0],
                    "id": "github-diagnostic-multiline-query",
                    "raw_query": f"foo{separator}bar",
                    "semantic_identity": (
                        "github-repository-search-v1:syntax:" \
                        f"foo{separator}bar"
                    ),
                    "intent_class": "brand",
                    "lifecycle": "diagnostic",
                    "kpi_role": "diagnostic",
                    "rationale": "Synthetic unprojectable multiline query.",
                    "activated_at": "2026-07-31T00:00:00Z",
                }
                self.registry["queries"].append(query)
                self.write_registry()
                with self.assertRaisesRegex(AuditError, "line separators"):
                    validate(self.head, self.base, self.audited_at)
                self.registry["queries"].pop()

    def test_github_registry_raw_query_rejects_html_syntax(self) -> None:
        for raw_query in (
            "python <em> compiler",
            'python <details title="<"> compiler',
            "python <?audit?> compiler",
            "python <!AUDIT> compiler",
            "python <![CDATA[audit]]> compiler",
            "python <!-- compiler",
            "python --> compiler",
        ):
            with self.subTest(raw_query=raw_query):
                self.registry["queries"].append(
                    {
                        **self.registry["queries"][0],
                        "id": "github-diagnostic-html-query",
                        "raw_query": raw_query,
                        "semantic_identity": (
                            "github-repository-search-v1:syntax:"
                            + raw_query.lower()
                        ),
                        "intent_class": "brand",
                        "lifecycle": "diagnostic",
                        "kpi_role": "diagnostic",
                        "rationale": "Synthetic unprojectable HTML query.",
                        "activated_at": "2026-07-31T00:00:00Z",
                    }
                )
                self.write_registry()
                with self.assertRaisesRegex(AuditError, "HTML tag or comment"):
                    validate(self.head, self.base, self.audited_at)
                self.registry["queries"].pop()

    def test_github_registry_raw_query_rejects_control_or_formatting_characters(
        self,
    ) -> None:
        for control in (
            "\x00",
            "\x01",
            "\t",
            "\x1f",
            "\x7f",
            "\x80",
            "\x9f",
            "\u200b",
            "\u202e",
            "\u2066",
            "\ufeff",
        ):
            with self.subTest(control=repr(control)):
                self.registry["queries"].append(
                    {
                        **self.registry["queries"][0],
                        "id": "github-diagnostic-control-query",
                        "raw_query": f"python{control}compiler",
                        "semantic_identity": (
                            "github-repository-search-v1:syntax:control-query"
                        ),
                        "intent_class": "brand",
                        "lifecycle": "diagnostic",
                        "kpi_role": "diagnostic",
                        "rationale": "Synthetic unprojectable control query.",
                        "activated_at": "2026-07-31T00:00:00Z",
                    }
                )
                self.write_registry()
                with self.assertRaisesRegex(AuditError, "control characters"):
                    validate(self.head, self.base, self.audited_at)
                self.registry["queries"].pop()

    def test_history_raw_query_rejects_embedded_backticks(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        path.write_text(
            path.read_text().replace(
                "`python aot compiler` | 7",
                "`python `aot` compiler` | 7",
                1,
            )
        )
        self.refresh_checkpoint()
        with self.assertRaisesRegex(AuditError, "embedded backticks"):
            validate(self.head, self.base, self.audited_at)

    def test_registry_version_rejects_json_boolean(self) -> None:
        self.registry["registry_version"] = True
        self.write_registry()
        with self.assertRaisesRegex(AuditError, "registry_version must be an integer"):
            validate(self.head, self.base, self.audited_at)

    def test_github_surface_contract_is_immutable(self) -> None:
        self.registry["surfaces"]["github_repository_search"] = {
            **GITHUB_SURFACE_CONTRACT,
            "result_window": 10,
        }
        self.write_registry()
        with self.assertRaisesRegex(AuditError, "surface contract was rewritten"):
            validate(self.head, self.base, self.audited_at)

    def test_github_surface_window_requires_an_integer(self) -> None:
        self.registry["surfaces"]["github_repository_search"] = {
            **GITHUB_SURFACE_CONTRACT,
            "result_window": 50.0,
        }
        self.write_registry()
        with self.assertRaisesRegex(AuditError, "surface result_window must be an integer"):
            validate(self.head, self.base, self.audited_at)

    def test_google_surface_contract_is_immutable(self) -> None:
        self.registry["surfaces"]["google_web"] = {
            **GOOGLE_SURFACE_CONTRACT,
            "transport": "url_inspection",
        }
        self.write_registry()
        with self.assertRaisesRegex(AuditError, "Google measurement surface"):
            validate(self.head, self.base, self.audited_at)

    def test_github_query_rejects_non_diagnostic_qualifiers(self) -> None:
        measurement = self.registry["measurements"][0]
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        original_row = (
            "| 2026-07-31T00:00:00Z | `python aot compiler` | 7 | +12 | 50 | 240 |"
        )
        for qualified_query in (
            "repo:rotnov/pycc",
            "user:rotnov",
            "org:openai",
            "language:python",
            "stars:>10",
        ):
            with self.subTest(qualified_query=qualified_query):
                qualified_row = (
                    "| 2026-07-31T00:00:00Z | "
                    f"`{qualified_query}` | 7 | — | 50 | 240 |"
                )
                path.write_text(self.head_visibility.replace(original_row, qualified_row))
                self.registry["queries"][0]["raw_query"] = qualified_query
                self.registry["queries"][0]["semantic_identity"] = (
                    "github-repository-search-v1:syntax:" + qualified_query
                )
                measurement["request_parameters"]["q"] = qualified_query
                self.write_registry()
                self.refresh_checkpoint()
                with self.assertRaisesRegex(AuditError, "qualifier policy"):
                    validate(self.head, self.base, self.audited_at)

    def test_registry_era_row_requires_replay_metadata(self) -> None:
        self.registry["measurements"] = []
        self.write_registry()
        with self.assertRaisesRegex(AuditError, "lacks trusted replay metadata"):
            validate(self.head, self.base, self.audited_at)

    def test_new_row_cannot_claim_legacy_status_from_its_timestamp(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        current_row = (
            "| 2026-07-31T00:00:00Z | `python aot compiler` | 7 | +12 | 50 | 240 |\n"
        )
        legacy_claim = (
            "| 2026-07-30T14:14:23Z | `python aot compiler` | 19 | 0 | 50 | 240 |"
        )
        markdown = path.read_text().replace(current_row, "")
        path.write_text(
            markdown.replace(
                "\n\n## GitHub traffic history",
                f"\n{legacy_claim}\n\n## GitHub traffic history",
            )
        )
        self.registry["measurements"] = []
        self.write_registry()
        self.refresh_checkpoint()
        with self.assertRaisesRegex(AuditError, "lacks trusted replay metadata"):
            validate(self.head, self.base, self.audited_at)

    def test_registry_preserves_trusted_base_measurements(self) -> None:
        (self.base / "docs" / "SEARCH_VISIBILITY.md").write_text(
            self.head_visibility
        )
        (self.base / "docs" / "SEARCH_QUERY_REGISTRY.json").write_text(
            json.dumps(self.registry, indent=2) + "\n"
        )
        self.registry["measurements"][0]["ordered_corpus_sha256"] = "b" * 64
        self.write_registry()
        with self.assertRaisesRegex(AuditError, "trusted base measurements"):
            validate(self.head, self.base, self.audited_at)

    def test_registry_preserves_trusted_base_queries(self) -> None:
        self.registry["queries"][0]["rationale"] += " Rewritten."
        self.write_registry()
        with self.assertRaisesRegex(AuditError, "trusted base queries"):
            validate(self.head, self.base, self.audited_at)

    def test_ai_native_query_remains_excluded_authorship_evidence(self) -> None:
        query = self.registry["queries"][1]
        query["lifecycle"] = "active"
        query["retired_at"] = None
        query["kpi_role"] = "product_acquisition"
        self.write_registry()
        with self.assertRaisesRegex(AuditError, "intent/KPI role"):
            validate(self.head, self.base, self.audited_at)

    def test_registry_enforces_every_intent_kpi_role(self) -> None:
        incompatible = (
            ("product_category", "diagnostic"),
            ("category_version", "diagnostic"),
            ("task_output", "diagnostic"),
            ("brand", "product_acquisition"),
            ("metadata_diagnostic", "product_acquisition"),
            ("topic_diagnostic", "product_acquisition"),
            ("competitive_category", "product_acquisition"),
            ("authorship_narrative", "product_acquisition"),
            ("unreviewed_intent", "diagnostic"),
        )
        query = self.registry["queries"][0]
        for intent_class, kpi_role in incompatible:
            with self.subTest(intent_class=intent_class, kpi_role=kpi_role):
                query["intent_class"] = intent_class
                query["kpi_role"] = kpi_role
                self.write_registry()
                with self.assertRaisesRegex(AuditError, "intent/KPI role"):
                    validate(self.head, self.base, self.audited_at)

    def test_checkpoints_preserve_the_trusted_base_prefix(self) -> None:
        base_rows = history_rows(
            (self.base / "docs" / "SEARCH_VISIBILITY.md").read_text()
        )
        base_checkpoint = {
            "checkpoint_version": 1,
            "surfaces": {
                "github_repository_search": [
                    {
                        "required_prefix_rows": len(base_rows),
                        "sha256": history_digest(base_rows),
                    }
                ]
            },
        }
        (
            self.base / "docs" / "SEARCH_VISIBILITY_CHECKPOINTS.json"
        ).write_text(json.dumps(base_checkpoint, indent=2) + "\n")
        with self.assertRaisesRegex(AuditError, "trusted base prefix"):
            validate(self.head, self.base, self.audited_at)

    def test_checkpoint_version_rejects_json_boolean(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY_CHECKPOINTS.json"
        document = json.loads(path.read_text())
        document["checkpoint_version"] = True
        path.write_text(json.dumps(document, indent=2) + "\n")
        with self.assertRaisesRegex(AuditError, "checkpoint_version must be an integer"):
            validate(self.head, self.base, self.audited_at)

    def test_registry_era_row_requires_correct_rank_delta(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        path.write_text(path.read_text().replace("| 7 | +12 |", "| 7 | nonsense |"))
        self.refresh_checkpoint()
        with self.assertRaisesRegex(AuditError, "rank delta disagrees"):
            validate(self.head, self.base, self.audited_at)

    def test_registry_replay_metadata_requires_valid_types_and_ranges(self) -> None:
        mutations = {
            "result window type": (
                "result_window",
                50.0,
                "result_window must be an integer",
            ),
            "returned_results type": ("returned_results", False, "must be an integer"),
            "returned_results range": (
                "returned_results",
                51,
                "outside its result window",
            ),
            "api_total relationship": (
                "api_total",
                49,
                "smaller than returned_results",
            ),
            "target_rank type": ("target_rank", False, "must be an integer"),
            "target_rank lower bound": (
                "target_rank",
                0,
                "outside returned results",
            ),
            "target_rank upper bound": (
                "target_rank",
                51,
                "outside returned results",
            ),
            "incomplete_results type": (
                "incomplete_results",
                0,
                "must be boolean",
            ),
            "corpus digest": (
                "ordered_corpus_sha256",
                "not-a-digest",
                "must be lowercase SHA-256",
            ),
        }
        original = dict(self.registry["measurements"][0])
        for name, (field, value, message) in mutations.items():
            with self.subTest(name=name):
                self.registry["measurements"][0] = {**original, field: value}
                self.write_registry()
                with self.assertRaisesRegex(AuditError, message):
                    validate(self.head, self.base, self.audited_at)

    def test_measurement_per_page_requires_an_integer(self) -> None:
        self.registry["measurements"][0]["request_parameters"]["per_page"] = 50.0
        self.write_registry()
        with self.assertRaisesRegex(AuditError, "request per_page must be an integer"):
            validate(self.head, self.base, self.audited_at)

    def test_complete_measurement_requires_the_complete_result_window(self) -> None:
        measurement = self.registry["measurements"][0]
        measurement["returned_results"] = 1
        measurement["target_rank"] = None
        self.write_registry()
        with self.assertRaisesRegex(AuditError, "incomplete result window"):
            validate(self.head, self.base, self.audited_at)

    def test_incomplete_response_cannot_produce_rank_evidence(self) -> None:
        self.registry["measurements"][0]["incomplete_results"] = True
        self.write_registry()
        with self.assertRaisesRegex(AuditError, "cannot produce rank evidence"):
            validate(self.head, self.base, self.audited_at)

    def test_future_observation_is_rejected(self) -> None:
        future = "9999-12-31T23:59:59Z"
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        path.write_text(path.read_text().replace("2026-07-31T00:00:00Z", future))
        self.registry["measurements"][0]["observed_at"] = future
        self.write_registry()
        self.refresh_checkpoint()
        with self.assertRaisesRegex(AuditError, "cannot be in the future"):
            validate(self.head, self.base, self.audited_at)

    def test_duplicate_observation_key_is_rejected(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        duplicate = (
            "| 2026-07-31T00:00:00Z | `python aot compiler` | 7 | 0 | 50 | 240 |"
        )
        path.write_text(
            path.read_text().replace(
                "\n\n## GitHub traffic history",
                f"\n{duplicate}\n\n## GitHub traffic history",
            )
        )
        self.refresh_checkpoint()
        with self.assertRaisesRegex(AuditError, "observation keys must be unique"):
            validate(self.head, self.base, self.audited_at)

    def test_empty_history_timestamp_is_rejected(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        path.write_text(
            path.read_text().replace(
                "\n\n## GitHub traffic history",
                "\n|  | `python aot compiler` | 1 | +6 | 3 | 3 |"
                "\n\n## GitHub traffic history",
            )
        )
        with self.assertRaisesRegex(AuditError, "history observed_at"):
            validate(self.head, self.base, self.audited_at)

    def test_backdated_append_is_rejected(self) -> None:
        path = self.head / "docs" / "SEARCH_VISIBILITY.md"
        path.write_text(
            path.read_text().replace(
                "\n\n## GitHub traffic history",
                "\n| 2026-07-30T00:00:00Z | `python aot compiler` | 8 | -1 | 50 | 230 |"
                "\n\n## GitHub traffic history",
            )
        )
        self.refresh_checkpoint()
        with self.assertRaisesRegex(AuditError, "timestamps must be nondecreasing"):
            validate(self.head, self.base, self.audited_at)


class SearchVisibilityAuditEntrypointTests(unittest.TestCase):
    """Exercise the real CLI entrypoint end-to-end through a subprocess.

    The mutation tests above import and call ``validate()`` directly. These
    tests execute the actual ``if __name__ == "__main__"`` wiring of
    ``check_search_visibility_audit.py`` so that disabling ``main()`` would
    leave at least one test red.
    """

    def setUp(self) -> None:
        self.repository_root = Path(__file__).resolve().parent.parent

    def _run_entrypoint(self, head_root: Path, base_root: Path) -> subprocess.CompletedProcess:
        return subprocess.run(
            [
                sys.executable,
                "-B",
                str(AUDIT_PATH),
                "--head-root",
                str(head_root),
                "--base-root",
                str(base_root),
            ],
            capture_output=True,
            text=True,
        )

    def test_entrypoint_passes_against_the_clean_tree(self) -> None:
        result = self._run_entrypoint(
            self.repository_root, self.repository_root
        )
        self.assertEqual(
            result.returncode,
            0,
            f"audit exited {result.returncode}\nstdout:\n{result.stdout}"
            f"\nstderr:\n{result.stderr}",
        )
        self.assertIn("Trusted search visibility audit passed.", result.stdout)

    def test_entrypoint_fails_on_a_corrupted_bootstrap(self) -> None:
        with tempfile.TemporaryDirectory(
            prefix="pycc-search-entrypoint-"
        ) as temporary:
            root = Path(temporary)
            docs = root / "docs"
            docs.mkdir(parents=True)
            for name in (
                "SEARCH_VISIBILITY.md",
                "SEARCH_QUERY_REGISTRY.json",
                "SEARCH_VISIBILITY_CHECKPOINTS.json",
                "ROADMAP.md",
            ):
                (docs / name).write_text(
                    (self.repository_root / "docs" / name).read_text()
                )
            visibility = (docs / "SEARCH_VISIBILITY.md").read_text()
            # Corrupt the first history row so the file's bootstrap digest no
            # longer matches the bound checkpoint, breaking the audit.
            original_row = (
                "| 2026-07-24T23:02:03Z | `pycc` | >50 | — | 50 | — |"
            )
            corrupted_row = (
                "| 2026-07-24T23:02:03Z | `pycc` | >50 | — | 49 | — |"
            )
            self.assertIn(original_row, visibility)
            (docs / "SEARCH_VISIBILITY.md").write_text(
                visibility.replace(original_row, corrupted_row, 1)
            )
            result = self._run_entrypoint(root, root)
            self.assertNotEqual(
                result.returncode,
                0,
                "audit unexpectedly passed against a corrupted bootstrap",
            )
            self.assertIn(
                "trusted search visibility audit failed:", result.stderr
            )


if __name__ == "__main__":
    unittest.main()
