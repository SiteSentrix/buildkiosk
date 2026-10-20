#!/usr/bin/env bash
# CI only: import the Developer ID certificate into a temporary keychain so codesign can use it.
#
# Inputs (env): APPLE_CERTIFICATE (base64 .p12), APPLE_CERTIFICATE_PASSWORD, KEYCHAIN_PASSWORD (optional; random if
# unset), RUNNER_TEMP (or TMPDIR).
# Outputs: MACOS_SIGN_IDENTITY (SHA-1 of the "Developer ID Application" identity) and MACOS_KEYCHAIN, appended to
# $GITHUB_ENV when running in Actions, otherwise printed.
#
# The caller (contrib/macos/sign.sh) checks that the secrets exist first; this script fails if the certificate does not
# import or holds no Developer ID Application identity. Delete the keychain afterwards (`security delete-keychain`).
set -euo pipefail
umask 077

: "${APPLE_CERTIFICATE:?APPLE_CERTIFICATE is not set}"

TMP="${RUNNER_TEMP:-${TMPDIR:-/tmp}}"
KEYCHAIN="$TMP/buildkiosk-signing.keychain-db"
CERT="$TMP/buildkiosk-signing.p12"
KC_PASS="${KEYCHAIN_PASSWORD:-$(openssl rand -hex 24)}"

trap 'rm -f "$CERT"' EXIT
printf '%s' "$APPLE_CERTIFICATE" | base64 --decode >"$CERT"
security create-keychain -p "$KC_PASS" "$KEYCHAIN"
# Lock again after six hours; a release job never runs that long.
security set-keychain-settings -lut 21600 "$KEYCHAIN"
security unlock-keychain -p "$KC_PASS" "$KEYCHAIN"
security import "$CERT" -P "${APPLE_CERTIFICATE_PASSWORD:-}" -T /usr/bin/codesign -T /usr/bin/security -t cert -f pkcs12 -k "$KEYCHAIN"
rm -f "$CERT"
# Let codesign use the key without a prompt nobody can answer on a runner.
security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$KC_PASS" "$KEYCHAIN" >/dev/null
# Put the keychain on the search list (keeping the existing ones) so codesign and notarytool find it.
# shellcheck disable=SC2046
security list-keychains -d user -s "$KEYCHAIN" $(security list-keychains -d user | tr -d '"')

IDENTITY="$(security find-identity -v -p codesigning "$KEYCHAIN" | awk '/Developer ID Application/ { print $2; exit }')"
if [ -z "$IDENTITY" ]; then
  security find-identity -v -p codesigning "$KEYCHAIN" >&2
  echo "error: no 'Developer ID Application' identity in APPLE_CERTIFICATE" >&2
  exit 1
fi
echo "Imported signing identity $IDENTITY"
if [ -n "${GITHUB_ENV:-}" ]; then
  { echo "MACOS_SIGN_IDENTITY=$IDENTITY"; echo "MACOS_KEYCHAIN=$KEYCHAIN"; } >>"$GITHUB_ENV"
else
  echo "MACOS_SIGN_IDENTITY=$IDENTITY"
  echo "MACOS_KEYCHAIN=$KEYCHAIN"
fi
