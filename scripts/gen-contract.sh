#!/usr/bin/env bash
# @author wzxgqtl 2026-11-20
# 契约文件是唯一类型源；两端生成文件必须一起提交。
set -euo pipefail
cd "$(dirname "$0")/../mobile"
pnpm exec quicktype --src-lang schema --lang rust --derive-debug --derive-partial-eq --top-level Contract -o ../desktop/src/contract/mod.rs ../shared/contract/protocol.json
rustfmt --edition 2024 ../desktop/src/contract/mod.rs
pnpm exec json2ts ../shared/contract/protocol.json src/generated/contract.ts
cd ..
node scripts/stamp-contract.mjs
