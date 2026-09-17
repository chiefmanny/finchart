#!/usr/bin/env bash
# Build the finchart WASM module into web/pkg/ for the static demo.
#
# One-time setup:
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-pack
#
# Then serve this directory over HTTP (WASM won't load from file://):
#   ./build.sh && python3 -m http.server --directory .
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
cd "$here/.."

wasm-pack build finchart-web --release --target web --out-dir "$here/pkg"

echo
echo "Built web/pkg. Serve the demo with:"
echo "  python3 -m http.server --directory \"$here\""
