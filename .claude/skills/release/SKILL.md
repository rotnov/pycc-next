---
name: release
description: Use when cutting a pycc-next release (version bump, changelog, tag, GitHub Release with binaries).
---

# Release

1. `main` is green in CI and `cargo test --release --workspace` passes locally.
2. Run the `bench` skill; put the table in the release notes.
3. Bump `version` in the root `Cargo.toml`; `cargo build --release` to refresh `Cargo.lock`.
4. Add `## [X.Y.Z] - YYYY-MM-DD` to `CHANGELOG.md` (Added / Changed / Fixed / Known gaps -
   copy open rows from `docs/DEVIATIONS.md` into Known gaps). The release workflow uses
   this section as the release notes.
5. Commit, push, then `git tag vX.Y.Z && git push origin vX.Y.Z`.
6. The `release` workflow tests, packages `pycc-vX.Y.Z-x86_64-linux.tar.gz` (+ sha256) and
   creates the GitHub Release. Verify the release exists and the tarball runs:
   download, `./pycc version`, build `bench/py/fib.py`.
7. Publish a launch post (`blog-post` skill).
