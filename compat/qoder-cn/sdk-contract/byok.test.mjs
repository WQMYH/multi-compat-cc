import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { mkdir, mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';

const helper = fileURLToPath(new URL('../../../src-tauri/resources/qoder/qoder-auth-host.mjs', import.meta.url));
const cliPath = fileURLToPath(new URL('./byok-fixture.mjs', import.meta.url));
const providerId = 'cc-' + 'b'.repeat(64);
const configuration = protocol => ({ providerId, displayName: 'Fixture CC provider', type: 'openai-compatible',
  baseUrl: 'https://fixture.invalid/v1', protocol, apiKey: 'synthetic-upstream-key', authType: protocol === 'anthropic' ? 'api-key' : 'bearer',
  model: 'model-A', models: [{ model: 'model-A', displayName: 'A', contextWindow: 120000 }, { model: 'model-B', capabilities: { vision: true } }],
});
async function run(root, input, mode) {
  const scratchDir = join(root, 'scratch'); await mkdir(scratchDir, { recursive: true });
  const child = spawn(process.execPath, [helper], { windowsHide: true,
    env: { ...process.env, NODE_OPTIONS: undefined, TEMP: scratchDir, TMP: scratchDir, TMPDIR: scratchDir,
      QODER_BYOK_FIXTURE_MODE: mode, QODERCN_PERSONAL_ACCESS_TOKEN: 'synthetic-ambient-token' }, stdio: ['pipe', 'pipe', 'pipe'] });
  let stdout = '', stderr = ''; child.stdout.on('data', b => stdout += b); child.stderr.on('data', b => stderr += b);
  const closed = once(child, 'close');
  child.stdin.end(JSON.stringify({ cliPath, scratchDir, operation: 'byok', action: 'sync', providerId,
    source: { type: 'cli', configDir: root }, config: configuration('openai-responses'), ...input }));
  const [exitCode] = await closed;
  assert.ok(!stdout.includes('synthetic-upstream-key') && !stderr.includes('synthetic-upstream-key'));
  assert.deepEqual(await readdir(scratchDir), []);
  const started = await readFile(join(root, 'started.json'), 'utf8').catch(() => null);
  if (started) assert.throws(() => process.kill(JSON.parse(started).pid, 0), e => e.code === 'ESRCH');
  return { exitCode, reply: JSON.parse(stdout) };
}
async function directory(t) { const root = await mkdtemp(join(tmpdir(), 'cc-qoder-byok-')); t.after(() => rm(root, { recursive: true, force: true })); return root; }

test('source25 configured BYOK directory survives a system-only account catalog without invented selection values', { timeout: 30000 }, async t => {
  const root = await directory(t), result = await run(root, {}, 'system-only');
  assert.equal(result.exitCode, 0);
  assert.deepEqual(result.reply.byok.nativeModelIds, ['model-A', 'model-B']);
  assert.deepEqual(result.reply.byok.modelValues, []);
  assert.equal(result.reply.byok.catalogSource, 'native_byok_configs');
  assert.equal(result.reply.byok.desktopDefaultChanged, false);
});

test('source25 reconciliation reads existing native config without credentials or mutation and rejects mismatches', { timeout: 30000 }, async t => {
  const root = await directory(t), config = configuration('openai-responses');
  const native = JSON.stringify({ globalModel: 'original-desktop-model', providers: { [providerId]: config } });
  await writeFile(join(root, 'native.json'), native);
  const { apiKey: _key, ...expected } = config;
  const matched = await run(root, { action: 'reconcile', config: expected }, 'system-only');
  assert.equal(matched.exitCode, 0);
  assert.equal(matched.reply.byok.readOnlyReconciled, true);
  assert.deepEqual(matched.reply.byok.modelValues, []);
  const mismatch = await run(root, { action: 'reconcile', config: { ...expected, authType: 'api-key' } }, 'system-only');
  assert.equal(mismatch.exitCode, 1);
  assert.equal(mismatch.reply.nativeEffect, 'none');
  assert.equal(await readFile(join(root, 'native.json'), 'utf8'), native);
  const calls = (await readFile(join(root, 'calls.jsonl'), 'utf8')).trim().split('\n').map(line => JSON.parse(line).method);
  assert.ok(calls.every(method => ['daemon.initialize', 'daemon.account_info', 'daemon.list_byok_configs', 'daemon.get_models', 'daemon.shutdown'].includes(method)));
});

test('real bundled SDK creates, changes provider default and deletes without changing desktop model or foreign providers', { timeout: 30000 }, async t => {
  for (const protocol of ['openai', 'openai-responses', 'anthropic']) {
    const root = await directory(t), config = configuration(protocol);
    const first = await run(root, { config });
    assert.equal(first.exitCode, 0); assert.equal(first.reply.byok.defaultModelId, 'model-A');
    assert.equal(first.reply.byok.desktopDefaultChanged, false);
    assert.equal(first.reply.byok.modelValues[1].value, `native-opaque:${providerId}:model-B`);
    const update = await run(root, { expectedUserId: 'fixture-native-user', config: { ...config, model: 'model-B' } });
    assert.equal(update.reply.byok.defaultModelId, 'model-B');
    let native = JSON.parse(await readFile(join(root, 'native.json')));
    assert.equal(native.globalModel, 'original-desktop-model'); assert.equal(native.providers.foreign.displayName, 'Preserve foreign row');
    const removed = await run(root, { action: 'delete', config: undefined, expectedUserId: 'fixture-native-user' });
    assert.equal(removed.reply.byok.deleted, true);
    native = JSON.parse(await readFile(join(root, 'native.json')));
    assert.deepEqual(Object.keys(native.providers), ['foreign']);
    const methods = (await readFile(join(root, 'calls.jsonl'), 'utf8')).trim().split('\n').map(s => JSON.parse(s).method);
    assert.equal(methods.filter(m => m === 'daemon.create_byok_config').length, 1);
    assert.equal(methods.filter(m => m === 'daemon.update_byok_config').length, 1);
    assert.ok(!methods.some(m => /check|validate|session|prompt|usage/.test(m)));
  }
});

test('account drift and unbound existing IDs stop before mutation; invalid input never starts native SDK', { timeout: 30000 }, async t => {
  const root = await directory(t);
  assert.equal((await run(root, {})).exitCode, 0);
  const before = await readFile(join(root, 'native.json'), 'utf8');
  const changed = await run(root, { expectedUserId: 'fixture-native-user' }, 'changed-account');
  assert.equal(changed.reply.code, 'QODER_ACCOUNT_IDENTITY_CHANGED'); assert.equal(changed.reply.nativeEffect, 'none');
  const collision = await run(root, {}); assert.equal(collision.reply.code, 'QODER_BYOK_EXISTING_ID_UNBOUND');
  assert.equal(await readFile(join(root, 'native.json'), 'utf8'), before);
  const clean = await directory(t);
  const invalid = await run(clean, { providerId: 'foreign' }); assert.equal(invalid.reply.nativeEffect, 'none');
  assert.equal(await readFile(join(clean, 'started.json'), 'utf8').catch(() => null), null);
});

test('lost acknowledgment remains an unknown effect, never a successful model switch', { timeout: 30000 }, async t => {
  for (const mode of ['ack-lost']) {
    const root = await directory(t), result = await run(root, {}, mode);
    assert.equal(result.exitCode, 1); assert.equal(result.reply.nativeEffect, 'unknown');
    assert.equal(result.reply.byok, undefined);
    const native = JSON.parse(await readFile(join(root, 'native.json')));
    assert.equal(native.providers[providerId].model, 'model-A', 'native mutation really happened before the failed receipt');
  }
});
