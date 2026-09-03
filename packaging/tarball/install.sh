#!/bin/sh
# Install RustPad from the release tarball. Run as your user for a per-user
# install under ~/.local, or with sudo for a system-wide install under /usr/local.
set -eu
cd "$(dirname "$0")"
if [ "$(id -u)" -eq 0 ]; then
  bin=/usr/local/bin; share=/usr/local/share
else
  bin="$HOME/.local/bin"; share="$HOME/.local/share"
fi
install -Dm755 rustpad "$bin/rustpad"
install -Dm644 com.tsubaie.rustpad.desktop "$share/applications/com.tsubaie.rustpad.desktop"
install -Dm644 com.tsubaie.rustpad.svg "$share/icons/hicolor/scalable/apps/com.tsubaie.rustpad.svg"
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$share/applications" || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q "$share/icons/hicolor" 2>/dev/null || true
echo "Installed rustpad to $bin/rustpad"
case ":$PATH:" in *":$bin:"*) ;; *) echo "Note: $bin is not on your PATH." ;; esac
