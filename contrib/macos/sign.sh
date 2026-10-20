#!/usr/bin/env bash
# CI only: sign a macOS binary with the Developer ID Application certificate and notarize it with Apple.
#
#   contrib/macos/sign.sh PATH/TO/buildkiosk
#
# Inputs (env): APPLE_CERTIFICATE (base64 .p12), APPLE_CERTIFICATE_PASSWORD, APPLE_ID, APPLE_PASSWORD (app-specific),
# APPLE_TEAM_ID, and optionally KEYCHAIN_PASSWORD.
#
# Missing secrets: prints a warning, leaves the binary unsigned and exits 0, so a build without the signing environment
# still produces working (unsigned) archives. Any other failure (a certificate that does not import, a rejected
# notarization) fails the run: a half-signed release must not go out.
#
# Steps: import the certificate into a temporary keychain (import-cert.sh); codesign with the hardened runtime and a
# secure timestamp; submit a zip of the binary to Apple's notary service and wait for the verdict. A bare binary cannot
# be stapled, so Gatekeeper looks the notarization up online the first time the file is opened.
set -euo pipefail

BIN="${1:?usage: sign.sh PATH/TO/binary}"
[ -f "$BIN" ] || { echo "error: $BIN not found" >&2; exit 1; }
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

warn() {
  if [ -n "${GITHUB_ACTIONS:-}" ]; then echo "::warning::$*"; else echo "warning: $*" >&2; fi
}

missing=()
for v in APPLE_CERTIFICATE APPLE_CERTIFICATE_PASSWORD APPLE_ID APPLE_PASSWORD APPLE_TEAM_ID; do
  [ -n "${!v:-}" ] || missing+=("$v")
done
if [ "${#missing[@]}" -gt 0 ]; then
  warn "macOS signing secrets not set (${missing[*]}): $(basename "$BIN") is NOT signed or notarized"
  exit 0
fi

"$HERE/import-cert.sh" >/dev/null
# import-cert.sh reports through $GITHUB_ENV in Actions; read the identity back from the keychain here instead, so this
# script also works outside Actions.
KEYCHAIN="${RUNNER_TEMP:-${TMPDIR:-/tmp}}/buildkiosk-signing.keychain-db"
IDENTITY="$(security find-identity -v -p codesigning "$KEYCHAIN" | awk '/Developer ID Application/ { print $2; exit }')"
[ -n "$IDENTITY" ] || { echo "error: signing identity not found after import" >&2; exit 1; }

echo "==> codesign $(basename "$BIN")"
codesign --force --sign "$IDENTITY" --keychain "$KEYCHAIN" --options runtime --timestamp "$BIN"
codesign --verify --strict --verbose=2 "$BIN"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
ZIP="$WORK/notarize.zip"
ditto -c -k --keepParent "$BIN" "$ZIP"

echo "==> notarizing (this can take a few minutes)"
out="$(xcrun notarytool submit "$ZIP" --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" \
  --team-id "$APPLE_TEAM_ID" --wait --timeout 1h --output-format json)" || true
echo "$out"
id="$(printf '%s' "$out" | plutil -extract id raw -o - - 2>/dev/null || true)"
status="$(printf '%s' "$out" | plutil -extract status raw -o - - 2>/dev/null || true)"
if [ "$status" != "Accepted" ]; then
  if [ -n "$id" ]; then
    xcrun notarytool log "$id" --apple-id "$APPLE_ID" --password "$APPLE_PASSWORD" --team-id "$APPLE_TEAM_ID" || true
  fi
  echo "error: notarization of $(basename "$BIN") failed (status: ${status:-unknown})" >&2
  exit 1
fi

# The ticket is published when the status is Accepted; ask Apple whether it is there.
codesign --verify --strict --check-notarization --verbose=2 "$BIN" \
  || warn "$(basename "$BIN") was accepted by the notary service but the notarization check did not confirm it yet"
codesign --display --verbose=2 "$BIN" 2>&1 | grep -E '^(Identifier|Authority|Timestamp|Runtime|CodeDirectory)' || true
echo "Signed and notarized $(basename "$BIN")"
