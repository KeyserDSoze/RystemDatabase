import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../../', import.meta.url));
const file = relative => readFileSync(root + relative, 'utf8');

test('TDD RED: GHCR owner must match lowercase GitHub account', () => {
  const workflow = file('.github/workflows/release.yml');
  assert.match(workflow, /ghcr\.io\/keyserdsoze\/rystemdatabase/);
  assert.doesNotMatch(workflow, /ghcr\.io\/keysersdoze\//);
});

test('TDD RED: bash latest lookup must include prereleases', () => {
  assert.match(file('scripts/install.sh'), /releases\?per_page=1/);
});

test('TDD RED: PowerShell latest lookup must include prereleases', () => {
  assert.match(file('scripts/install.ps1'), /releases\?per_page=1/);
});
