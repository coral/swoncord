#!/usr/bin/env bash
set -euo pipefail

version=$(python3 scripts/ci/version.py)
if [[ "${GITHUB_REF:-}" != "refs/tags/v$version" ]]; then
  echo 'GitHub releases require a matching version tag' >&2
  exit 1
fi
tag="v$version"
dmg="target/ci/dist/Swoncord-$version-macos-arm64.dmg"
shopt -s nullglob
assets=(target/ci/dist/*)
if [[ ${#assets[@]} != 1 || "${assets[0]}" != "$dmg" || ! -s "$dmg" ]]; then
  echo "Expected exactly one signed release asset: $dmg" >&2
  exit 1
fi

# Upload to a draft first so a failed upload never publishes an empty release.
if existing=$(gh release view "$tag" --json isDraft --jq .isDraft); then
  if [[ "$existing" != true ]]; then
    echo "Release $tag is already public; refusing to replace its assets" >&2
    exit 1
  fi
  # An interrupted upload can be retried, but do not publish unexpected assets
  # that may have been attached to a pre-existing draft.
  existing_assets=$(gh release view "$tag" --json assets --jq '.assets[].name')
  if [[ -n "$existing_assets" && "$existing_assets" != "$(basename "$dmg")" ]]; then
    echo 'Draft contains unexpected assets; refusing to publish' >&2
    exit 1
  fi
else
  gh release create "$tag" --verify-tag --draft --generate-notes --title "Swoncord $version"
fi
gh release upload "$tag" "$dmg" --clobber
gh release edit "$tag" --draft=false --latest
