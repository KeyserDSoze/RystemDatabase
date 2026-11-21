#!/usr/bin/env node
import { readFile } from 'node:fs/promises';
import { spawn } from 'node:child_process';
import { platformAsset } from '../lib/platform.mjs';
import { ensureInstalled } from '../lib/install.mjs';

async function main() {
  const manifest = JSON.parse(await readFile(new URL('../package.json', import.meta.url), 'utf8'));
  if (process.argv.includes('--wrapper-info')) {
    console.log('rystemdb npm launcher ' + manifest.version + ' (' + platformAsset() + ')');
    return;
  }
  const binary = await ensureInstalled(manifest.version);
  const child = spawn(binary, process.argv.slice(2), { stdio: 'inherit', windowsHide: true });
  for (const signal of ['SIGINT', 'SIGTERM']) {
    process.on(signal, () => { if (!child.killed) child.kill(signal); });
  }
  child.on('error', error => {
    console.error('rystemdb: ' + error.message);
    process.exitCode = 1;
  });
  child.on('exit', (code, signal) => {
    process.exitCode = code ?? (signal ? 1 : 0);
  });
}

main().catch(error => {
  console.error('rystemdb: ' + error.message);
  process.exitCode = 1;
});
