#!/usr/bin/env bash
# Resolves and validates the release tag for .github/workflows/release.yml.
#
#   INPUT_VERSION=vX.Y.Z scripts/release_tag.sh   # workflow_dispatch: tag to create
#   scripts/release_tag.sh                        # tag push: GITHUB_REF_NAME
#
# The tag must be vMAJOR.MINOR.PATCH with an optional -suffix; a dispatched
# version must not exist yet. Appends TAG=... to $GITHUB_ENV. Every job of
# the workflow runs it, so each validates the tag on its own.
set -euo pipefail

tag="${INPUT_VERSION:-${GITHUB_REF_NAME:-}}"
if ! [[ "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]]; then
    echo "bad tag: '$tag'" >&2
    exit 1
fi
if [ -n "${INPUT_VERSION:-}" ] && git ls-remote --exit-code --tags origin "refs/tags/$tag" >/dev/null; then
    echo "tag $tag already exists" >&2
    exit 1
fi
echo "TAG=$tag" >> "$GITHUB_ENV"
