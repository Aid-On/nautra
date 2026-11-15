#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# Builds every example guest to target/wasm/<name>.wasm.
set -euo pipefail
cd "$(dirname "$0")/.."
almide="${ALMIDE:-.tools/almide/almide}"
mkdir -p target/wasm
for source in examples/*/*.almd; do
  name="$(basename "$source" .almd)"
  "$almide" build "$source" --target wasm -o "target/wasm/$name.wasm" 2>&1 | grep -E "^Built|error" || true
done
# The same program as a WASI 0.2 component, to cover that path too.
"$almide" build examples/motd/motd.almd --target wasm --component -o target/wasm/motd.component.wasm 2>&1 | grep -E "^Built|error" || true
