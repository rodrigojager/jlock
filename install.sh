#!/bin/sh
# Installs jlock on Linux or macOS, from a GitHub Release:
#
#   curl -fsSL https://raw.githubusercontent.com/rodrigojager/jlock/main/install.sh | sh
#
# Settings, all optional:
#   JLOCK_VERSION      the release to install, such as v0.1.0 (default: the latest one)
#   JLOCK_INSTALL_DIR  where to put the binary (default: ~/.local/bin, or /usr/local/bin when it is
#                      writable and ~/.local/bin is not in PATH)
#   JLOCK_BASE_URL     where the release files are (default: the project's GitHub Releases; file://
#                      works too, for tests)
#   JLOCK_NO_PATH=1    do not print the PATH advice
#   JLOCK_TARGET       the target triple to download, instead of the detected one
#
# It never uses sudo and never edits your shell profile: if the folder is not in PATH it prints the
# line to add.

set -eu

REPO="rodrigojager/jlock"
BASE_URL="${JLOCK_BASE_URL:-https://github.com/$REPO/releases/download}"
API_URL="${JLOCK_API_URL:-https://api.github.com/repos/$REPO/releases/latest}"

say() { printf 'jlock install: %s\n' "$*"; }
die() {
    printf 'jlock install: %s\n' "$*" >&2
    exit 1
}

# fetch URL DEST
fetch() {
    if command -v curl >/dev/null 2>&1; then
        curl -fsSL --retry 2 -o "$2" "$1"
    elif command -v wget >/dev/null 2>&1; then
        case "$1" in
        file://*) cp "${1#file://}" "$2" ;;
        *) wget -q -O "$2" "$1" ;;
        esac
    else
        die "needs curl or wget"
    fi
}

detect_target() {
    if [ -n "${JLOCK_TARGET:-}" ]; then
        printf '%s\n' "$JLOCK_TARGET"
        return
    fi
    os="$(uname -s)"
    arch="$(uname -m)"
    case "$arch" in
    x86_64 | amd64) arch="x86_64" ;;
    aarch64 | arm64) arch="aarch64" ;;
    *) die "no build for this architecture: $arch" ;;
    esac
    case "$os" in
    Linux) printf '%s\n' "$arch-unknown-linux-musl" ;;
    Darwin) printf '%s\n' "$arch-apple-darwin" ;;
    *) die "no build for this system: $os (on Windows, use install.ps1)" ;;
    esac
}

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d ' ' -f 1
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d ' ' -f 1
    else
        die "needs sha256sum or shasum to check the download"
    fi
}

in_path() {
    case ":$PATH:" in
    *":$1:"*) return 0 ;;
    *) return 1 ;;
    esac
}

target="$(detect_target)"

version="${JLOCK_VERSION:-}"
if [ -z "$version" ]; then
    tmp_api="$(mktemp)"
    fetch "$API_URL" "$tmp_api" || die "cannot ask GitHub for the latest release ($API_URL)"
    version="$(sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$tmp_api" | head -n 1)"
    rm -f "$tmp_api"
    [ -n "$version" ] || die "no release found; set JLOCK_VERSION"
fi
case "$version" in
v*) ;;
*) version="v$version" ;;
esac
number="${version#v}"
asset="jlock-$number-$target.tar.gz"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

say "downloading $asset ($version)"
fetch "$BASE_URL/$version/$asset" "$tmp/$asset" || die "cannot download $BASE_URL/$version/$asset"
fetch "$BASE_URL/$version/SHA256SUMS" "$tmp/SHA256SUMS" || die "cannot download SHA256SUMS"

want="$(awk -v f="$asset" '$2 == f || $2 == "*" f { print $1 }' "$tmp/SHA256SUMS" | head -n 1)"
[ -n "$want" ] || die "$asset is not listed in SHA256SUMS; nothing was installed"
got="$(sha256_of "$tmp/$asset")"
[ "$got" = "$want" ] || die "checksum mismatch for $asset (expected $want, got $got); nothing was installed"

tar -xzf "$tmp/$asset" -C "$tmp" jlock || die "cannot unpack $asset"

if [ -n "${JLOCK_INSTALL_DIR:-}" ]; then
    dir="$JLOCK_INSTALL_DIR"
else
    dir="$HOME/.local/bin"
    if ! in_path "$dir" && [ -d /usr/local/bin ] && [ -w /usr/local/bin ]; then
        dir="/usr/local/bin"
    fi
fi
mkdir -p "$dir" || die "cannot create $dir"

# Copy next to the old one, then rename over it: a jlock running right now keeps its file.
cp "$tmp/jlock" "$dir/.jlock.new.$$" || die "cannot write in $dir (set JLOCK_INSTALL_DIR to a folder you own)"
chmod 755 "$dir/.jlock.new.$$"
mv -f "$dir/.jlock.new.$$" "$dir/jlock"

installed="$("$dir/jlock" --version)" || die "the installed $dir/jlock does not run"
say "installed $installed in $dir"

if [ "${JLOCK_NO_PATH:-}" != "1" ] && ! in_path "$dir"; then
    say "$dir is not in PATH. Add this line to your shell profile (~/.profile, ~/.bashrc or ~/.zshrc):"
    # shellcheck disable=SC2016 # $PATH is meant to stay literal in the printed line
    printf '  export PATH="%s:$PATH"\n' "$dir"
fi
say "check it with: jlock doctor"
