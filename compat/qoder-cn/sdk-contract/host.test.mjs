import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdir, mkdtemp, readFile, readdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const helper = fileURLToPath(new URL('../../../src-tauri/resources/qoder/qoder-auth-host.mjs', import.meta.url));
const cliPath = fileURLToPath(new URL('./host-fixture.mjs', import.meta.url));

async function run(input, extraEnv = {}) {
  const directory = await mkdtemp(join(tmpdir(), 'qoder-host-contract-'));
  const scratchDir = join(directory, 'owned-scratch');
  await mkdir(scratchDir);
  const source = input.source || { type: 'pat_env', envVar: 'QODER_HOST_TEST_PAT' };
  const child = spawn(process.execPath, [helper], {
    windowsHide: true,
    env: { ...process.env, TEMP: scratchDir, TMP: scratchDir, TMPDIR: scratchDir,
      NODE_OPTIONS: undefined, QODER_HOST_TEST_PAT: 'synthetic-host-token',
      QODERCN_PERSONAL_ACCESS_TOKEN: 'synthetic-ambient-token-must-not-override-cli',
      QODER_HOST_FIXTURE_DIRECTORY: directory, ...extraEnv },
    stdio: ['pipe', 'pipe', 'pipe'],
  });
  let stdout = '', stderr = '';
  child.stdout.on('data', bytes => { stdout += bytes.toString(); });
  child.stderr.on('data', bytes => { stderr += bytes.toString(); });
  const closed = once(child, 'close');
  child.stdin.end(JSON.stringify({ cliPath, scratchDir, ...input, source: source.type === 'cli' ? { type: 'cli', configDir: directory } : source }));
  const [code] = await closed;
  assert.ok(!stdout.includes('synthetic-host-token') && !stderr.includes('synthetic-host-token'));
  assert.ok(!stdout.includes('must-not-escape'));
  const native = await readFile(join(directory, 'child.json'), 'utf8').catch(() => null);
  if (native) {
    const observed = JSON.parse(native);
    assert.throws(() => process.kill(observed.pid, 0), error => error.code === 'ESRCH');
    assert.equal(observed.temporaryDirectory, scratchDir);
    if (source.type === 'pat_env') assert.ok(observed.configDir.startsWith(scratchDir));
  }
  assert.deepEqual(await readdir(scratchDir), [], 'normal completion must remove SDK diagnostics and temporary payloads');
  return { code, reply: JSON.parse(stdout), nativeStarted: !!native };
}

test('bundled production helper verifies PAT and explicit native CLI identity without exposing extra account fields', { timeout: 20000 }, async () => {
  for (const source of [{ type: 'pat_env', envVar: 'QODER_HOST_TEST_PAT' }, { type: 'cli' }]) {
    const result = await run({ source });
    assert.equal(result.code, 0);
    assert.deepEqual(result.reply, { ok: true, account: { userId: 'fixture-A', name: 'Fixture A' } });
    assert.equal(result.nativeStarted, true);
  }
});

test('changed identity and empty identity are explicit failures, not successful login', { timeout: 20000 }, async () => {
  const changed = await run({ expectedUserId: 'fixture-B' });
  assert.equal(changed.code, 1);
  assert.equal(changed.reply.code, 'QODER_ACCOUNT_IDENTITY_CHANGED');
  const empty = await run({}, { QODER_HOST_FIXTURE_EMPTY: '1' });
  assert.equal(empty.code, 1);
  assert.equal(empty.reply.code, 'QODER_ACCOUNT_UNVERIFIED');
});

test('missing credential and invalid source reject before launching any native process', { timeout: 20000 }, async () => {
  const missing = await run({ source: { type: 'pat_env', envVar: 'QODER_HOST_MISSING_PAT' } }, { QODER_HOST_MISSING_PAT: undefined });
  assert.equal(missing.reply.code, 'QODER_CREDENTIAL_MISSING');
  assert.equal(missing.nativeStarted, false);
  const invalid = await run({ source: { type: 'api_key', token: 'must-not-be-accepted' } });
  assert.equal(invalid.reply.code, 'QODER_AUTH_SOURCE_REQUIRED');
  assert.equal(invalid.nativeStarted, false);
});

test('source23 native login required is actionable without leaking SDK text or starting inference', { timeout: 20000 }, async () => {
  const result = await run({ source: { type: 'cli' } }, { QODER_HOST_FIXTURE_AUTH_REQUIRED: '1' });
  assert.equal(result.code, 1);
  assert.equal(result.nativeStarted, true);
  assert.deepEqual(result.reply, { ok: false, code: 'AUTH_REQUIRED', retryable: false });
});
