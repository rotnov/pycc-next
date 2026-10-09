# CodeContests corpus subset

A pinned, vendored subset of the DeepMind CodeContests dataset, used to measure
how much real competitive-programming Python `pycc` compiles unchanged. See
`docs/TESTING.md` for how the measurement is run and reported.

These are plain checked-in files, not a submodule: the corpus is part of the
repository tree so that a clean clone can run the metric with no network
access.

## Provenance and licensing

Provenance, the pinned dataset revision, the citation and the upstream
acknowledgements are in `NOTICE`; the dataset's CC BY 4.0 licence text is in
`LICENSE`. The repository's own MIT licence is unaffected and still covers
everything outside this directory.

Dataset content is third-party text from the public internet and is treated as
untrusted. A `solution.py` here is only ever executed by
`scripts/check_corpus_compile_rate.py` in the subprocess it starts on purpose.

## What is vendored

* `problems/<NNN>-<slug>/` — 200 steering problems.
* `holdout/<NNN>-<slug>/` — 100 holdout problems, excluded from the default
  denominator and reported only under `--include-holdout`. **Do not inspect or
  optimise against the holdout set.** Its only purpose is to show that
  improvements measured on the steering set generalise; reading it while
  choosing what to implement destroys that.
* `solution.py` — one PYTHON3 solution per problem, verbatim apart from
  line-ending normalisation.
* `tests.json` — that problem's public and private cases, packed into one file
  (`{"cases": [{"input": ..., "output": ...}]}`). Cases are never truncated,
  and `generated_tests` is excluded entirely.
* `manifest.json` — the pinned revision, each shard's name/size/sha256, a
  sha256 for every vendored file, and the per-problem records.

Problem statements, descriptions, incorrect solutions, non-Python solutions and
dataset metadata are **not** vendored.

## Regenerating

One command, from the repository root, with `pyarrow` available on the
interpreter that runs it (it is deliberately not a repository dependency —
`scripts/check_corpus_compile_rate.py` must never import it):

```sh
python3 scripts/select_codecontests_corpus.py
```

Re-running it against the same pinned revision rewrites the same bytes, so a
regeneration that changes nothing leaves the working tree clean. The pinned
shards themselves are downloaded into `--cache-dir` (`.cache/codecontests` by
default, several hundred megabytes), which `.gitignore` excludes: the cache is
a local download, never tracked evidence. The selector fails loudly with a
per-filter rejection histogram rather than silently shrinking the corpus.
