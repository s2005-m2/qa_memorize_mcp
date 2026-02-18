import { createRequire } from 'module';
import { readFileSync, writeFileSync, unlinkSync, existsSync } from 'fs';
import { gunzipSync } from 'zlib';
import { join, dirname } from 'path';

const require = createRequire(import.meta.url);

const platforms = [
  'qa-memorize-mcp-win-x64',
  'qa-memorize-mcp-linux-x64',
  'qa-memorize-mcp-darwin-x64',
  'qa-memorize-mcp-darwin-arm64',
];

let pkgDir = null;
for (const pkg of platforms) {
  try {
    const pkgJson = require.resolve(`${pkg}/package.json`);
    pkgDir = dirname(pkgJson);
    break;
  } catch {}
}

if (!pkgDir) process.exit(0);

const modelDir = join(pkgDir, 'bin', 'embedding_model');

for (const file of ['model_ort.onnx.gz', 'tokenizer.json.gz']) {
  const gz = join(modelDir, file);
  const out = gz.slice(0, -3);
  if (!existsSync(gz)) continue;
  if (existsSync(out)) { unlinkSync(gz); continue; }
  process.stderr.write(`postinstall: decompressing ${file}\n`);
  writeFileSync(out, gunzipSync(readFileSync(gz)));
  unlinkSync(gz);
}
