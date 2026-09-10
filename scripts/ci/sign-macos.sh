#!/usr/bin/env bash
set -euo pipefail
set +x
umask 077

for name in APPLE_CERTIFICATE_BASE64 APPLE_CERTIFICATE_PASSWORD APPLE_API_KEY_BASE64 APPLE_API_KEY_ID APPLE_API_ISSUER_ID; do
  if [[ -z "${!name:-}" ]]; then echo "Missing release secret: $name" >&2; exit 1; fi
done
: "${RUNNER_TEMP:?This script runs on the GitHub macOS signing runner}"
version=$(python3 scripts/ci/version.py)
work="$RUNNER_TEMP/swoncord-signing"
keychain="$work/signing.keychain-db"
mkdir -p "$work/app" target/ci/dist
cleanup() {
  security delete-keychain "$keychain" >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT

ditto -x -k unsigned/Swoncord.app.zip "$work/app"
app="$work/app/Swoncord.app"
[[ $(lipo -archs "$app/Contents/MacOS/swoncord") == arm64 ]]
[[ $(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$app/Contents/Info.plist") == "$version" ]]
[[ $(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$app/Contents/Info.plist") == com.oblique.media.swoncord ]]

printf '%s' "$APPLE_CERTIFICATE_BASE64" | base64 --decode > "$work/certificate.p12"
keychain_password=$(openssl rand -hex 32)
echo "::add-mask::$keychain_password"
security create-keychain -p "$keychain_password" "$keychain"
security set-keychain-settings -lut 2700 "$keychain"
security unlock-keychain -p "$keychain_password" "$keychain"
security import "$work/certificate.p12" -k "$keychain" -P "$APPLE_CERTIFICATE_PASSWORD" -T /usr/bin/codesign >/dev/null
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$keychain_password" "$keychain" >/dev/null
security list-keychains -d user -s "$keychain"
rm "$work/certificate.p12"
unset APPLE_CERTIFICATE_BASE64 APPLE_CERTIFICATE_PASSWORD keychain_password

identity=$(security find-identity -v -p codesigning "$keychain" | awk '/"Developer ID Application:/ {print $2}')
if [[ ! "$identity" =~ ^[0-9A-Fa-f]{40}$ ]]; then
  echo 'Expected one valid Developer ID Application identity in the certificate export' >&2
  exit 1
fi
codesign --force --sign "$identity" --keychain "$keychain" --options runtime --timestamp \
  --entitlements Resources/entitlements.plist "$app"
codesign --verify --deep --strict "$app"

printf '%s' "$APPLE_API_KEY_BASE64" | base64 --decode > "$work/notary-key.p8"
xcrun notarytool store-credentials swoncord-notarization --keychain "$keychain" \
  --key "$work/notary-key.p8" --key-id "$APPLE_API_KEY_ID" --issuer "$APPLE_API_ISSUER_ID" >/dev/null
rm "$work/notary-key.p8"
unset APPLE_API_KEY_BASE64 APPLE_API_KEY_ID APPLE_API_ISSUER_ID

# Notarizing the outer disk image also notarizes the signed app it contains.
dmg="target/ci/dist/Swoncord-$version-macos-arm64.dmg"
bash scripts/ci/create-macos-dmg.sh "$app" "$dmg"
codesign --force --sign "$identity" --keychain "$keychain" --timestamp "$dmg"
codesign --verify --strict "$dmg"
submit_status=0
xcrun notarytool submit "$dmg" --keychain "$keychain" \
  --keychain-profile swoncord-notarization --wait --timeout 30m --output-format json > "$work/result.json" || submit_status=$?
status=$(jq -r '.status // empty' "$work/result.json" 2>/dev/null || true)
if [[ "$submit_status" != 0 || "$status" != Accepted ]]; then
  submission=$(jq -r '.id // empty' "$work/result.json" 2>/dev/null || true)
  if [[ -n "$submission" ]]; then
    if xcrun notarytool log "$submission" --keychain "$keychain" \
      --keychain-profile swoncord-notarization "$work/notary-log.json"; then
      jq '.issues' "$work/notary-log.json"
    fi
  fi
  echo 'Apple did not accept the notarization submission' >&2
  exit 1
fi
xcrun stapler staple "$dmg"
xcrun stapler validate "$dmg"
spctl --assess --type open --context context:primary-signature --verbose=2 "$dmg"
