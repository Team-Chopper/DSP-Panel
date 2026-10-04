#!/usr/bin/env bash
# Build the release package for the Ubuntu server:
# Flutter web client + Linux server binary + install scripts.
#
#   tool/build_dist.sh [x64|arm64]
#
# Works on Linux, macOS and Windows (Git Bash); the server binary is
# cross-compiled for Linux. Result: dist/dsp-panel-<version>-linux-<arch>.tar.gz

set -euo pipefail

ARCH="${1:-x64}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

VERSION="$(sed -n 's/^version:[[:space:]]*//p' pubspec.yaml | tr -d '\r')"
NAME="dsp-panel-$VERSION-linux-$ARCH"
OUT="$ROOT/dist/$NAME"

echo "==> Building $NAME"
rm -rf "$OUT" "$ROOT/dist/$NAME.tar.gz"
mkdir -p "$OUT"

echo "==> Web client"
flutter pub get
# No CDN and no service worker: the panel must work without internet access
# and with a self-signed certificate.
flutter build web --release --no-web-resources-cdn --pwa-strategy=none --no-wasm-dry-run
cp -r build/web "$OUT/web"

echo "==> Server (linux-$ARCH)"
(
  cd server
  dart pub get
  dart compile exe bin/dsp_server.dart --target-os=linux --target-arch="$ARCH" -o "$OUT/dsp-server"
)

cp deploy/install.sh deploy/uninstall.sh deploy/dsp-panel.service deploy/config.example.yaml "$OUT/"
chmod +x "$OUT/install.sh" "$OUT/uninstall.sh" "$OUT/dsp-server"

tar -C "$ROOT/dist" --mode='u+rwX,go+rX' -czf "$ROOT/dist/$NAME.tar.gz" "$NAME"
echo "==> dist/$NAME.tar.gz"
