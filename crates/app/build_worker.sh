#!/bin/sh
# Builds the PNG rasterization worker (resvg) for the trunk pipeline.
# wasm-bindgen regenerates its output every run; the conditional copy keeps
# unchanged files untouched so trunk's watcher does not rebuild forever.
set -e
cd "$(dirname "$0")"
cargo build -p render-worker --target wasm32-unknown-unknown --release \
  --target-dir target
wasm-bindgen --target no-modules --out-dir target/worker-gen --out-name render_worker \
  target/wasm32-unknown-unknown/release/render_worker.wasm
mkdir -p static/worker
for f in render_worker.js render_worker_bg.wasm render_worker_bg.wasm.d.ts; do
  if [ ! -f "static/worker/$f" ] || ! cmp -s "target/worker-gen/$f" "static/worker/$f"; then
    cp "target/worker-gen/$f" "static/worker/$f"
  fi
done
