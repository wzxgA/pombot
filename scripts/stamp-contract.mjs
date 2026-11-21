import { readFileSync, writeFileSync } from 'node:fs';

const author = '// @author wzxgqtl 2026-11-20\n// 此文件由 scripts/gen-contract.sh 生成，请修改 shared/contract/protocol.json。\n';
const rustPath = 'desktop/src/contract/mod.rs';
const rust = readFileSync(rustPath, 'utf8').replace(/^\/\/ Example[\s\S]*?\n\nuse serde/, 'use serde');
writeFileSync(rustPath, author + rust);

const tsPath = 'mobile/src/generated/contract.ts';
const ts = readFileSync(tsPath, 'utf8')
  .replace(/^\/\* eslint-disable \*\/\n/, '')
  .replaceAll('/**', '/** @author wzxgqtl 2026-11-20\n');
writeFileSync(tsPath, `// @author wzxgqtl 2026-11-20\n// 此文件由 scripts/gen-contract.sh 生成，请修改 shared/contract/protocol.json。\n${ts}`);
