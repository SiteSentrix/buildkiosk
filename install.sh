#!/bin/sh
# Installs a BuildKiosk release: downloads the archive for this machine from the project's GitHub releases,
# checks it against the published SHA-256 checksums, and puts the `buildkiosk` program in place.
#
#   curl -fsSL https://raw.githubusercontent.com/SiteSentrix/buildkiosk/main/install.sh | sh
#   sh install.sh --version v0.1.0 --prefix /opt/buildkiosk
#   sudo sh install.sh --service            # Linux with systemd: also a service user, data folder and unit
#
# Read it before you run it; it is short. It changes nothing outside the folders named below.
# Options:
#   --version VERSION   a release tag such as v0.1.0 (default: the newest release, including pre-releases)
#   --prefix DIR        install the program in DIR/bin (default: /usr/local as root, otherwise ~/.local)
#   --service           Linux, as root: create the `buildkiosk` user, /var/lib/buildkiosk and a systemd unit
#   --start             with --service: enable the service and start it now
#   --dry-run           show what would be done, change nothing
#   -h, --help          this text
# Environment: BUILDKIOSK_REPO (owner/name), BUILDKIOSK_BASE_URL and BUILDKIOSK_API_URL (for testing).
set -eu

REPO="${BUILDKIOSK_REPO:-SiteSentrix/buildkiosk}"
BASE="${BUILDKIOSK_BASE_URL:-https://github.com/$REPO/releases/download}"
API="${BUILDKIOSK_API_URL:-https://api.github.com/repos/$REPO}"

version=""
prefix=""
service=0
start=0
dry=0

say() { printf '%s\n' "$*"; }
die() { printf 'install.sh: %s\n' "$*" >&2; exit 1; }

usage() {
    cat <<'TEXT'
Usage: sh install.sh [options]
  --version VERSION   a release tag such as v0.1.0 (default: the newest release, including pre-releases)
  --prefix DIR        install the program in DIR/bin (default: /usr/local as root, otherwise ~/.local)
  --service           Linux, as root: create the buildkiosk user, /var/lib/buildkiosk and a systemd unit
  --start             with --service: enable the service and start it now
  --dry-run           show what would be done, change nothing
  -h, --help          this text
Environment: BUILDKIOSK_REPO (owner/name), BUILDKIOSK_BASE_URL and BUILDKIOSK_API_URL (for testing).
TEXT
}

while [ $# -gt 0 ]; do
    case "$1" in
        --version) [ $# -ge 2 ] || die "--version needs a value"; version="$2"; shift 2 ;;
        --prefix) [ $# -ge 2 ] || die "--prefix needs a value"; prefix="$2"; shift 2 ;;
        --service) service=1; shift ;;
        --start) service=1; start=1; shift ;;
        --dry-run) dry=1; shift ;;
        -h | --help) usage; exit 0 ;;
        *) die "unknown option: $1 (try --help)" ;;
    esac
done

# ---- this machine
os="$(uname -s)"
arch="$(uname -m)"
case "$os/$arch" in
    Linux/x86_64) target="x86_64-unknown-linux-musl" ;;
    Linux/aarch64 | Linux/arm64) target="aarch64-unknown-linux-musl" ;;
    Darwin/x86_64) target="x86_64-apple-darwin" ;;
    Darwin/arm64) target="aarch64-apple-darwin" ;;
    *) die "no release build for $os/$arch (Windows: download the .zip from the releases page)" ;;
esac

if [ -z "$prefix" ]; then
    if [ "$(id -u)" = "0" ]; then prefix="/usr/local"; else prefix="${HOME:?HOME is not set}/.local"; fi
fi

# ---- tools
if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL "$1"; }
    fetch_to() { curl -fsSL -o "$2" "$1"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -qO- "$1"; }
    fetch_to() { wget -qO "$2" "$1"; }
else
    die "needs curl or wget"
fi
if command -v sha256sum >/dev/null 2>&1; then
    sha256() { sha256sum "$1" | cut -d ' ' -f 1; }
elif command -v shasum >/dev/null 2>&1; then
    sha256() { shasum -a 256 "$1" | cut -d ' ' -f 1; }
else
    die "needs sha256sum or shasum to check the download"
fi

# ---- which release
if [ -z "$version" ]; then
    say "Looking for the newest release..."
    version="$(fetch "$API/releases?per_page=1" | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1)"
    [ -n "$version" ] || die "could not find a release; give one with --version"
fi
case "$version" in v*) tag="$version" ;; *) tag="v$version" ;; esac
plain="${tag#v}"
case "$plain" in *[!0-9A-Za-z.+-]* | "") die "that does not look like a version: $version" ;; esac
archive="buildkiosk-$plain-$target.tar.gz"

say "BuildKiosk $tag for $target"
say "  download  $BASE/$tag/$archive"
say "  install   $prefix/bin/buildkiosk"
if [ "$service" = 1 ]; then
    say "  service   user buildkiosk, data /var/lib/buildkiosk, unit /etc/systemd/system/buildkiosk.service$([ "$start" = 1 ] && printf ', start now')"
fi
if [ "$dry" = 1 ]; then
    say "(dry run: nothing was changed)"
    exit 0
fi

if [ "$service" = 1 ]; then
    [ "$os" = "Linux" ] || die "--service is for Linux with systemd"
    [ "$(id -u)" = "0" ] || die "--service needs root (try sudo)"
    command -v systemctl >/dev/null 2>&1 || die "--service needs systemd"
fi

# ---- download and check
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM
fetch_to "$BASE/$tag/$archive" "$tmp/$archive" || die "could not download $archive"
fetch_to "$BASE/$tag/SHA256SUMS" "$tmp/SHA256SUMS" || die "could not download SHA256SUMS"
want="$(awk -v f="$archive" '$2 == f || $2 == "*" f { print $1; exit }' "$tmp/SHA256SUMS")"
[ -n "$want" ] || die "$archive is not in SHA256SUMS"
got="$(sha256 "$tmp/$archive")"
[ "$want" = "$got" ] || die "checksum mismatch for $archive (expected $want, got $got); nothing was installed"
say "Checksum ok."

mkdir "$tmp/unpack"
tar -xzf "$tmp/$archive" -C "$tmp/unpack"
dir="$tmp/unpack/buildkiosk-$plain-$target"
[ -f "$dir/buildkiosk" ] || die "the archive does not contain the program"

# ---- install
mkdir -p "$prefix/bin"
cp "$dir/buildkiosk" "$prefix/bin/buildkiosk.new"
chmod 0755 "$prefix/bin/buildkiosk.new"
mv -f "$prefix/bin/buildkiosk.new" "$prefix/bin/buildkiosk"
"$prefix/bin/buildkiosk" version

if [ "$service" = 1 ]; then
    if ! id buildkiosk >/dev/null 2>&1; then
        useradd --system --home-dir /var/lib/buildkiosk --shell /usr/sbin/nologin buildkiosk
        say "Created the buildkiosk user."
    fi
    mkdir -p /var/lib/buildkiosk
    chown buildkiosk:buildkiosk /var/lib/buildkiosk
    chmod 0750 /var/lib/buildkiosk
    unit="/etc/systemd/system/buildkiosk.service"
    if [ -e "$unit" ]; then
        say "$unit already exists; left as it is."
    else
        sed "s#^ExecStart=/usr/local/bin/buildkiosk#ExecStart=$prefix/bin/buildkiosk#" "$dir/buildkiosk.service" > "$unit"
        say "Wrote $unit."
    fi
    systemctl daemon-reload
    if [ "$start" = 1 ]; then
        systemctl enable --now buildkiosk
        say "Started. It listens on 127.0.0.1:8080: put an HTTPS proxy in front of it (docs/deployment.md)."
    else
        say "Start it with: systemctl enable --now buildkiosk"
    fi
fi

case ":${PATH:-}:" in
    *":$prefix/bin:"*) ;;
    *) say "Note: $prefix/bin is not on your PATH." ;;
esac
say "Done. Try: buildkiosk demo --data ./demo-data && buildkiosk serve --data ./demo-data"
