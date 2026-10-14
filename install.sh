#!/bin/sh
# RusTXT quick install for Linux on x86_64:
#
#   curl -fsSL https://raw.githubusercontent.com/tsubaie/RusTXT/main/install.sh | sh
#
# Downloads the latest release built for your distro, checks it against the
# release's SHA256SUMS, and installs it with pacman, apt or dnf. On any other
# Linux it unpacks the tarball into ~/.local instead. Nothing is compiled and
# nothing outside the package is touched.
#
#   RUSTXT_VERSION=0.2.1    install that release instead of the latest
#   RUSTXT_INSTALL=tarball  use the tarball even when a package would fit
#   PREFIX=/opt/rustxt      where the tarball install goes (default ~/.local)
set -eu

repo="tsubaie/RusTXT"

say()  { printf '%s\n' "$*"; }
fail() { say "install.sh: $*" >&2; exit 1; }

fetch() {
  if command -v curl >/dev/null 2>&1; then curl -fsSL "$1" -o "$2"
  elif command -v wget >/dev/null 2>&1; then wget -q "$1" -O "$2"
  else fail "curl or wget is required"
  fi
}

# The release JSON is pretty-printed one key per line; python3 parses it
# properly when present, otherwise sed takes the first top-level tag_name.
latest_version() {
  if command -v python3 >/dev/null 2>&1; then
    python3 -I -c 'import json, sys
tag = json.load(open(sys.argv[1]))["tag_name"]
print(tag[1:] if tag.startswith("v") else tag)' "$1" 2>/dev/null && return 0
  fi
  sed -n 's/^[[:space:]]*"tag_name"[[:space:]]*:[[:space:]]*"v\{0,1\}\([^"]*\)".*/\1/p' "$1" | head -n 1
}

[ "$(uname -s)" = Linux ] || fail "prebuilt packages are Linux only. See https://github.com/$repo/blob/main/README.md#build-and-run"
[ "$(uname -m)" = x86_64 ] || fail "prebuilt packages are x86_64 only. See https://github.com/$repo/blob/main/README.md#build-and-run"

if [ "$(id -u)" -eq 0 ]; then sudo=""
elif command -v sudo >/dev/null 2>&1; then sudo=sudo
else sudo=""; fi

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

# Which release ---------------------------------------------------------------
version="${RUSTXT_VERSION:-}"
if [ -z "$version" ]; then
  fetch "https://api.github.com/repos/$repo/releases/latest" "$tmp/release.json"
  version="$(latest_version "$tmp/release.json")"
  [ -n "$version" ] || fail "could not work out the latest release"
fi
base="https://github.com/$repo/releases/download/v$version"

# Which package ---------------------------------------------------------------
kind="${RUSTXT_INSTALL:-}"
if [ -z "$kind" ]; then
  if command -v pacman >/dev/null 2>&1; then kind=pacman
  elif command -v apt-get >/dev/null 2>&1 && command -v dpkg >/dev/null 2>&1; then kind=deb
  elif command -v dnf >/dev/null 2>&1; then kind=rpm
  else kind=tarball
  fi
fi
case "$kind" in
  pacman)  file="rustxt-$version-1-x86_64.pkg.tar.zst" ;;
  deb)     file="rustxt_$version-1_amd64.deb" ;;
  rpm)     file="rustxt-$version-1.x86_64.rpm" ;;
  tarball) file="rustxt-$version-x86_64-linux.tar.gz" ;;
  *)       fail "RUSTXT_INSTALL must be pacman, deb, rpm or tarball" ;;
esac

# Download and verify ----------------------------------------------------------
say "Downloading RusTXT $version ($file)"
fetch "$base/$file" "$tmp/$file" || fail "could not download $base/$file (is v$version a published release?)"
fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS" || fail "could not download $base/SHA256SUMS"
expected="$(grep " $file\$" "$tmp/SHA256SUMS" | cut -d' ' -f1)"
[ -n "$expected" ] || fail "$file is not listed in SHA256SUMS"
actual="$(sha256sum "$tmp/$file" | cut -d' ' -f1)"
[ "$actual" = "$expected" ] || fail "checksum mismatch for $file"

# Install ----------------------------------------------------------------------
if [ "$kind" != tarball ] && [ -z "$sudo" ] && [ "$(id -u)" -ne 0 ]; then
  fail "installing a package needs root or sudo. Set RUSTXT_INSTALL=tarball for a per-user install."
fi
case "$kind" in
  pacman)  $sudo pacman -U --noconfirm "$tmp/$file" ;;
  deb)     $sudo apt-get install -y "$tmp/$file" ;;
  rpm)     $sudo dnf install -y "$tmp/$file" ;;
  tarball) tar -C "$tmp" -xzf "$tmp/$file" && PREFIX="${PREFIX:-$HOME/.local}" sh "$tmp/rustxt-$version/install.sh" ;;
esac

say ""
say "RusTXT $version is installed. Find it in your app menu or run: rustxt"
