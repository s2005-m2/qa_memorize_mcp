#!/usr/bin/env node
'use strict';

const path = require('path');
const fs = require('fs');
const { spawnSync } = require('child_process');
const zlib = require('zlib');

const PLATFORMS = {
  'win32-x64':    { pkg: 'qa-memorize-mcp-win-x64',    bin: 'memorize_mcp.exe' },
  'linux-x64':    { pkg: 'qa-memorize-mcp-linux-x64',   bin: 'memorize_mcp' },
  'darwin-x64':   { pkg: 'qa-memorize-mcp-darwin-x64',  bin: 'memorize_mcp' },
  'darwin-arm64': { pkg: 'qa-memorize-mcp-darwin-arm64', bin: 'memorize_mcp' },
};

const key = `${process.platform}-${process.arch}`;
const entry = PLATFORMS[key];
if (!entry) {
  console.error('Unsupported platform: ' + key);
  process.exit(1);
}

let pkgDir;
try {
  pkgDir = path.dirname(require.resolve(entry.pkg + '/package.json'));
} catch (e) {
  console.error(`Platform package ${entry.pkg} not found: ${e.message}`);
  process.exit(1);
}

const binDir = fs.realpathSync(path.join(pkgDir, 'bin'));
const binPath = path.join(binDir, entry.bin);
const modelDir = path.join(binDir, 'embedding_model');

for (const name of ['model_ort.onnx', 'tokenizer.json']) {
  const dest = path.join(modelDir, name);
  const src = dest + '.gz';
  if (!fs.existsSync(dest) && fs.existsSync(src)) {
    fs.writeFileSync(dest, zlib.gunzipSync(fs.readFileSync(src)));
  }
}

const args = process.argv.slice(2);
const finalArgs = args.includes('--model-dir')
  ? args
  : ['--model-dir', modelDir, ...args];

const result = spawnSync(binPath, finalArgs, {
  stdio: 'inherit',
  env: { ...process.env },
});

process.exit(result.status ?? 1);
