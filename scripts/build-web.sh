#!/usr/bin/env bash
# Builds the browser viewer: a WebGPU and a WebGL2 variant plus the loader page (plan D23).
#
# Usage: scripts/build-web.sh [out-dir]      (default: target/web)
# Serve the output directory with any static file server, e.g.
#   python3 -m http.server --directory target/web 8080
#
# Requires the wasm32-unknown-unknown target and a wasm-bindgen CLI whose version matches
# the wasm-bindgen crate in Cargo.lock.
set -euo pipefail

cd "$(dirname "$0")/.."
out="${1:-target/web}"
profile="${PROFILE:-release}"
artifact_dir="$profile"
[ "$profile" = "dev" ] && artifact_dir="debug"

expected="$(grep -A1 '^name = "wasm-bindgen"$' Cargo.lock | sed -n 's/^version = "\(.*\)"$/\1/p')"
actual="$(wasm-bindgen --version | awk '{print $2}')"
if [ "$expected" != "$actual" ]; then
  echo "wasm-bindgen CLI $actual does not match crate $expected" >&2
  exit 1
fi

rm -rf "$out"
mkdir -p "$out"
cp web/index.html "$out/"

for renderer in webgpu webgl2; do
  # Separate target directories keep both variants' build caches.
  target_dir="target/web-build/$renderer"
  cargo build -p aerocosm-viewer --target wasm32-unknown-unknown --profile "$profile" \
    --no-default-features --features "$renderer" --target-dir "$target_dir"
  wasm-bindgen --target web --no-typescript --out-dir "$out/$renderer" \
    "$target_dir/wasm32-unknown-unknown/$artifact_dir/aerocosm-viewer.wasm"
done

du -sh "$out"/*/
