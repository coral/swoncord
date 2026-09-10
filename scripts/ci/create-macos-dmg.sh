#!/usr/bin/env bash
set -euo pipefail

app=${1:?Usage: create-macos-dmg.sh APP OUTPUT.dmg}
output=${2:?Usage: create-macos-dmg.sh APP OUTPUT.dmg}
[[ -d "$app/Contents" && "$app" == *.app ]]
stage=$(mktemp -d "${TMPDIR:-/tmp}/swoncord-dmg.XXXXXX")
trap 'rm -rf "$stage"' EXIT
ditto "$app" "$stage/$(basename "$app")"
ln -s /Applications "$stage/Applications"
mkdir -p "$(dirname "$output")"
hdiutil create -volname Swoncord -srcfolder "$stage" -fs HFS+ -format UDZO -ov "$output"
hdiutil verify "$output"
