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
5. Commit and push. Then run the `release` workflow with `workflow_dispatch` on `main`,
   input `version: vX.Y.Z` (GitHub creates the tag; pushing tags from a cloud agent session
   is blocked by its git proxy). `git push origin vX.Y.Z` from a normal checkout also works.
6. The `release` workflow tests, packages `pycc-vX.Y.Z-x86_64-linux.tar.gz` (+ sha256)
   with `scripts/package.sh` (`bin/pycc`, `lib/pycc/libpycc_rt.a`, README, LICENSE,
   CHANGELOG), smoke-tests the unpacked tarball with `scripts/smoke.sh` (native, `--ext`,
   `--non-strict` vs CPython, cleared environment) and creates the GitHub Release.
   Verify the release exists and the tarball runs on a machine without the checkout:
   download, unpack, `scripts/smoke.sh pycc-vX.Y.Z-x86_64-linux`.
7. Publish a launch post (`blog-post` skill).
8. A broken pre-release (`vX.Y.Z-suffix`) is removed with the `release-delete` workflow
   (`workflow_dispatch` on `main`, input `tag`); it deletes the release and its tag and
   refuses final versions.
