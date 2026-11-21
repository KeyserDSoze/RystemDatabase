import test from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { platformAsset } from '../../npm/cli/lib/platform.mjs';
import { parseChecksum } from '../../npm/cli/lib/install.mjs';

test('TDD: platform mapping covers all release assets', () => {
  assert.equal(platformAsset('linux', 'x64'), 'rystemdb-linux-x64');
  assert.equal(platformAsset('linux', 'arm64'), 'rystemdb-linux-arm64');
  assert.equal(platformAsset('darwin', 'x64'), 'rystemdb-darwin-x64');
  assert.equal(platformAsset('darwin', 'arm64'), 'rystemdb-darwin-arm64');
  assert.equal(platformAsset('win32', 'x64'), 'rystemdb-windows-x64.exe');
  assert.throws(() => platformAsset('aix', 's390x'), /Unsupported platform/);
});

test('TDD: checksum lookup requires an exact asset name', () => {
  const hash = 'a'.repeat(64);
  assert.equal(parseChecksum(hash + '  rystemdb-linux-x64\n', 'rystemdb-linux-x64'), hash);
  assert.throws(() => parseChecksum(hash + '  evil-rystemdb-linux-x64\n', 'rystemdb-linux-x64'), /does not list/);
  assert.throws(() => parseChecksum('malformed', 'rystemdb-linux-x64'), /does not list/);
});

test('TDD: wrapper can show its platform without a release or network', () => {
  const script = fileURLToPath(new URL('../../npm/cli/bin/rystemdb.mjs', import.meta.url));
  const output = execFileSync(process.execPath, [script, '--wrapper-info'], { encoding: 'utf8' });
  assert.match(output, /rystemdb npm launcher 0\.1\.2/);
});

test('TDD RED: Bash installer supports macOS default Bash 3.2 syntax', async () => {
  const { readFileSync } = await import('node:fs');
  const script = fileURLToPath(new URL('../../scripts/install.sh', import.meta.url));
  const text = readFileSync(script, 'utf8');
  assert.doesNotMatch(text, /\$\{(?:actual|expected),,\}/,
    'Bash 4+ lowercase expansion is not supported by macOS system Bash 3.2');
  assert.match(text, /tr '\[:upper:\]' '\[:lower:\]'/,
    'checksum comparison should normalize case using portable tr');
});
