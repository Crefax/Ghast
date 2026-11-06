#!/usr/bin/env node
/**
 * Builds the browser client into dist/ (committed, so the Rust crate embeds it and `cargo build` needs
 * no Node):
 *
 *   dist/page.html    the page template (%APP% %WORKER% %DATA% %PACK% are filled in by the server)
 *   dist/app.js       the page script (src/main.ts)
 *   dist/worker.js    the mesher worker (src/mesher.worker.ts)
 *   dist/images.json  a JSON array: the non-block images the server must put in the texture pack
 *
 *   node build.mjs           build
 *   node build.mjs --check   build in memory and fail if dist/ differs (stale bundles)
 *   node build.mjs --dev     unminified bundles with inline source maps (not for committing)
 */
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import * as esbuild from 'esbuild';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const DIST = path.join(HERE, 'dist');
const check = process.argv.includes('--check');
const dev = process.argv.includes('--dev');

async function bundle(entry, format = 'iife') {
  const r = await esbuild.build({
    entryPoints: [path.join(HERE, 'src', entry)], bundle: true, write: false, format,
    minify: !dev, sourcemap: dev ? 'inline' : false,
    target: ['es2020', 'safari15'], legalComments: 'none', logLevel: 'warning', charset: 'utf8',
  });
  return Buffer.from(r.outputFiles[0].contents);
}

/** The image list comes from the model modules themselves: bundle images.ts for Node and run it. */
async function images() {
  const code = (await esbuild.build({
    entryPoints: [path.join(HERE, 'src', 'images.ts')], bundle: true, write: false, format: 'esm', platform: 'neutral', logLevel: 'warning',
  })).outputFiles[0].text;
  const mod = await import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`);
  return Buffer.from(JSON.stringify(mod.pageImages(), null, 1) + '\n');
}

const [app, worker, list] = await Promise.all([bundle('main.ts'), bundle('mesher.worker.ts'), images()]);
const out = {
  'page.html': fs.readFileSync(path.join(HERE, 'page.html')),
  'app.js': app,
  'worker.js': worker,
  'images.json': list,
};

if (check) {
  const stale = Object.entries(out).filter(([name, body]) => {
    const f = path.join(DIST, name);
    return !fs.existsSync(f) || !fs.readFileSync(f).equals(body);
  }).map(([name]) => name);
  if (stale.length) {
    console.error(`dist/ is stale (${stale.join(', ')}): run \`npm run build\` and commit dist/`);
    process.exit(1);
  }
  console.log('dist/ is up to date');
} else {
  fs.mkdirSync(DIST, { recursive: true });
  for (const [name, body] of Object.entries(out)) fs.writeFileSync(path.join(DIST, name), body);
  const kb = (b) => `${(b.length / 1024).toFixed(0)} KB`;
  console.log(`dist/: app.js ${kb(app)}, worker.js ${kb(worker)}, images.json ${JSON.parse(list).length} images${dev ? ' (dev)' : ''}`);
}
