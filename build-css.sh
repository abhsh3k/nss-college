#!/usr/bin/env bash
# Builds static/css/tailwind.css with the standalone Tailwind CLI (no Node needed).
#   bash build-css.sh            one-off build
#   bash build-css.sh --watch    rebuild whenever a template changes
set -euo pipefail
cd "$(dirname "$0")"

VERSION="v3.4.17"   # the project uses Tailwind v3 syntax; do not use the v4 CLI

case "$(uname -s)" in
  Linux)  OS=linux ;;
  Darwin) OS=macos ;;
  *) echo "Unsupported system: $(uname -s)" >&2; exit 1 ;;
esac
case "$(uname -m)" in
  x86_64)        ARCH=x64 ;;
  aarch64|arm64) ARCH=arm64 ;;
  *) echo "Unsupported CPU: $(uname -m)" >&2; exit 1 ;;
esac

BIN=./tailwindcss
if [ ! -x "$BIN" ]; then
  echo "Downloading Tailwind CLI $VERSION ($OS-$ARCH) ..."
  curl -fsSL -o "$BIN" "https://github.com/tailwindlabs/tailwindcss/releases/download/$VERSION/tailwindcss-$OS-$ARCH"
  chmod +x "$BIN"
fi

if [ "${1:-}" = "--watch" ]; then
  "$BIN" -c tailwind.config.js -i assets/input.css -o static/css/tailwind.css --watch
else
  "$BIN" -c tailwind.config.js -i assets/input.css -o static/css/tailwind.css --minify
fi
ls -lh static/css/tailwind.css
