#!/usr/bin/env bash
set -euo pipefail

cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
output="${1:-target/pages}"
target_dir="${CARGO_TARGET_DIR:-target}"
wasm_target=wasm32-unknown-unknown
bindgen_version="$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/"/, "", $3); print $3; exit }' Cargo.lock)"

if [[ "$(wasm-bindgen --version)" != "wasm-bindgen ${bindgen_version}" ]]; then
    printf 'Expected wasm-bindgen CLI %s (see Cargo.lock).\n' "$bindgen_version" >&2
    exit 1
fi

web_dependencies="$(cargo tree --locked --target "$wasm_target" --no-default-features --features web --edges normal --prefix none)"
if grep -E '^(async-opcua|tokio-modbus|rmcp) v' <<< "$web_dependencies"; then
    printf 'The browser build must exclude PLC and MCP runtimes.\n' >&2
    exit 1
fi

cargo build --locked --release --lib --target "$wasm_target" --no-default-features --features web
mkdir -p -- "$output/app/pkg"
cp -- web/index.html web/site.css web/tokens.css web/base.css "$output/"
cp -R -- web/assets web/app web/demo "$output/"
wasm-bindgen --target web --no-typescript --out-dir "$output/app/pkg" \
    "$target_dir/$wasm_target/release/factory_sim.wasm"


# Keep the JavaScript loader and WASM on the same release across browser caches.
app="$output/app"
revision="$(cat "$app/app.css" "$app/app.js" "$app/pkg/factory_sim.js" "$app/pkg/factory_sim_bg.wasm" | cksum | awk '{print $1}')"
sed -e "s|src=\"./app.js\"|src=\"./app.js?v=$revision\"|" \
    -e "s|href=\"./app.css\"|href=\"./app.css?v=$revision\"|" web/app/index.html > "$app/index.html"
