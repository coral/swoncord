#!/usr/bin/env bash
set -euo pipefail

: "${CARGO_BUILD_TARGET:=aarch64-apple-darwin}"
if [[ "$CARGO_BUILD_TARGET" != aarch64-apple-darwin ]]; then
  echo 'Swoncord packaging supports aarch64-apple-darwin only' >&2
  exit 1
fi
version=$(python3 scripts/ci/version.py)
cargo bundle --release --format osx --target "$CARGO_BUILD_TARGET"
git diff --exit-code -- Cargo.lock
app="target/$CARGO_BUILD_TARGET/release/bundle/osx/Swoncord.app"
binary="$app/Contents/MacOS/swoncord"
[[ $(lipo -archs "$binary") == arm64 ]]
[[ $(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$app/Contents/Info.plist") == "$version" ]]
/usr/libexec/PlistBuddy -c 'Print :NSAppleEventsUsageDescription' "$app/Contents/Info.plist"

# Rust dependencies are linked statically; the app must not depend on Homebrew
# libraries or other files that are only present on the build runner.
while IFS= read -r dependency; do
  case "$dependency" in
    /System/*|/usr/lib/*) ;;
    *) echo "Unexpected runtime dependency: $dependency" >&2; exit 1 ;;
  esac
done < <(otool -L "$binary" | tail -n +2 | sed -E 's/^[[:space:]]*//; s/ \(compatibility.*$//')

# Preserve the original icon and add the application license before signing.
cp assets/icon.icns "$app/Contents/Resources/icon.icns"
/usr/libexec/PlistBuddy -c 'Set :CFBundleIconFile icon.icns' "$app/Contents/Info.plist"
cp LICENSE.txt "$app/Contents/Resources/"
codesign --force --sign - --options runtime --entitlements Resources/entitlements.plist "$app"
codesign --verify --deep --strict "$app"
mkdir -p target/ci
ditto -c -k --sequesterRsrc --keepParent "$app" target/ci/Swoncord.app.zip
