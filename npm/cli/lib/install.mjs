import { createHash } from 'node:crypto';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { mkdir, readFile, rename, chmod, rm, writeFile } from 'node:fs/promises';
import { randomUUID } from 'node:crypto';
import { platformAsset } from './platform.mjs';

const REPOSITORY = 'https://github.com/KeyserDSoze/RystemDatabase';
const MAX_ASSET_SIZE = 100 * 1024 * 1024;

export function parseChecksum(manifest, filename) {
  for (const line of manifest.split(/\r?\n/)) {
    const match = /^([a-fA-F0-9]{64})\s+\*?(\S+)$/.exec(line.trim());
    if (match && match[2] === filename) return match[1].toLowerCase();
  }
  throw new Error('SHA256SUMS does not list ' + filename);
}

function sha256(buffer) {
  return createHash('sha256').update(buffer).digest('hex');
}

async function download(url) {
  const response = await fetch(url, { redirect: 'follow', signal: AbortSignal.timeout(60_000) });
  if (!response.ok) throw new Error('Download failed: HTTP ' + response.status + ' (' + url + ')');
  const size = Number(response.headers.get('content-length') || 0);
  if (size > MAX_ASSET_SIZE) throw new Error('Asset exceeds size limit');
  const buffer = Buffer.from(await response.arrayBuffer());
  if (buffer.length > MAX_ASSET_SIZE) throw new Error('Asset exceeds size limit');
  return buffer;
}

export async function ensureInstalled(version) {
  if (!/^0|[1-9]/.test(version) || !/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(version)) {
    throw new Error('Invalid package version');
  }
  const filename = platformAsset();
  const base = REPOSITORY + '/releases/download/v' + version + '/';
  const manifest = (await download(base + 'SHA256SUMS')).toString('utf8');
  const expected = parseChecksum(manifest, filename);
  const directory = join(homedir(), '.rystemdb', 'bin', 'v' + version);
  const destination = join(directory, filename);
  await mkdir(directory, { recursive: true });
  try {
    const current = await readFile(destination);
    if (sha256(current) === expected) return destination;
  } catch (error) {
    if (error.code !== 'ENOENT') throw error;
  }
  const binary = await download(base + filename);
  if (sha256(binary) !== expected) throw new Error('Checksum mismatch for ' + filename);
  const temporary = destination + '.tmp-' + randomUUID();
  try {
    await writeFile(temporary, binary, { mode: 0o755, flag: 'wx' });
    if (process.platform !== 'win32') await chmod(temporary, 0o755);
    await rm(destination, { force: true });
    await rename(temporary, destination);
  } finally {
    await rm(temporary, { force: true });
  }
  return destination;
}
