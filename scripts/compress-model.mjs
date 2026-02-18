import { createReadStream, createWriteStream, mkdirSync, statSync } from 'fs';
import { createGzip } from 'zlib';
import { pipeline } from 'stream/promises';
import { join } from 'path';

const args = process.argv.slice(2);
const get = (flag, def) => { const i = args.indexOf(flag); return i !== -1 ? args[i + 1] : def; };

const inputDir = get('--input-dir', 'embedding_model');
const outputDir = get('--output-dir', null);

if (!outputDir) { console.error('--output-dir is required'); process.exit(1); }

mkdirSync(outputDir, { recursive: true });

const files = ['model_ort.onnx', 'tokenizer.json'];

for (const file of files) {
  const src = join(inputDir, file);
  const dst = join(outputDir, file + '.gz');
  const before = statSync(src).size;
  await pipeline(createReadStream(src), createGzip(), createWriteStream(dst));
  const after = statSync(dst).size;
  console.log(`${file}: ${(before / 1024).toFixed(1)}KB → ${(after / 1024).toFixed(1)}KB`);
}
