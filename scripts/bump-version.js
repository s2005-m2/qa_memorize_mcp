#!/usr/bin/env node
'use strict';
const { readFileSync, writeFileSync } = require('fs');
const { join } = require('path');

const npmDir = join(__dirname, '..', 'npm');
const pkgs = [
  'qa-memorize-mcp',
  'qa-memorize-mcp-win-x64',
  'qa-memorize-mcp-linux-x64',
  'qa-memorize-mcp-darwin-x64',
  'qa-memorize-mcp-darwin-arm64',
];

const mainPkg = join(npmDir, 'qa-memorize-mcp', 'package.json');
const current = JSON.parse(readFileSync(mainPkg, 'utf8')).version;

let next = process.argv[2];
if (!next) {
  const [major, minor, patch] = current.split('.').map(Number);
  next = `${major}.${minor}.${patch + 1}`;
}

if (!/^\d+\.\d+\.\d+$/.test(next)) {
  process.stderr.write(`Invalid version: ${next}\n`);
  process.exit(1);
}

for (const name of pkgs) {
  const p = join(npmDir, name, 'package.json');
  const json = JSON.parse(readFileSync(p, 'utf8'));
  json.version = next;
  if (json.optionalDependencies) {
    for (const k of Object.keys(json.optionalDependencies)) {
      json.optionalDependencies[k] = next;
    }
  }
  writeFileSync(p, JSON.stringify(json, null, 2) + '\n');
}

process.stdout.write(`${current} -> ${next}\n`);
