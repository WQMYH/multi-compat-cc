import assert from 'node:assert/strict';
import { mkdtemp, mkdir, readFile, readdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { accessToken, accessTokenFromEnv, query, startDaemon } from '@qodercn-ai/qodercn-agent-sdk';

const nativeFixture = fileURLToPath(new URL('./daemon-fixture.mjs', import.meta.url));
const tokenVariable = 'QODER_SDK_CONTRACT_TOKEN';

async function setup(t, profile = 'A', mode = 'ok') {
  // Retain isolated diagnostics, including failing runs; never use the real CLI.
  const directory = await mkdtemp(join(tmpdir(), 'qoder-sdk-contract-'));
  const privateHome = join(directory, 'home');
  await mkdir(privateHome);
  const options = {
    auth: accessTokenFromEnv(tokenVariable),
    pathToQoderCLIExecutable: nativeFixture,
    initializeTimeoutMs: 3000,
    debugFile: join(directory, 'sdk.log'),
    env: {
      HOME: privateHome,
      USERPROFILE: privateHome,
      QODERCN_CONFIG_DIR: join(privateHome, '.qoder-cn'),
      QODER_CONFIG_DIR: join(privateHome, '.qoder'),
      QODERCN_PERSONAL_ACCESS_TOKEN: undefined,
      QODER_PERSONAL_ACCESS_TOKEN: undefined,
      QODERCN_SERVICE_ACCOUNT_KEY: undefined,
      QODER_SERVICE_ACCOUNT_KEY: undefined,
      QODER_SDK_FIXTURE_DIRECTORY: directory,
      QODER_SDK_FIXTURE_PROFILE: profile,
      QODER_SDK_FIXTURE_MODE: mode,
      [tokenVariable]: `synthetic-sdk-token-${profile}`,
    },
  };
  let daemon;
  let launchRejected = false;
  t.after(async () => {
    await daemon?.stop();
    const startedText = await readFile(join(directory, 'started.json'), 'utf8').catch(() => null);
    if (startedText) {
      const { pid } = JSON.parse(startedText);
      assert.throws(() => process.kill(pid, 0), error => error.code === 'ESRCH', 'the task-owned native fixture must be gone');
      const stoppedText = await readFile(join(directory, 'stopped.json'), 'utf8').catch(() => null);
      // Windows force termination cannot run the child's JavaScript exit hook.
      if (stoppedText) assert.equal(JSON.parse(stoppedText).code, 0);
      else assert.ok(launchRejected, 'normal shutdown must produce a clean exit record');
    }
    const log = await readFile(join(directory, 'sdk.log'), 'utf8').catch(() => '');
    if (startedText) assert.match(log, /\[Daemon\] spawned/, 'redaction checks must examine actual SDK diagnostics');
    assert.ok(!log.includes('synthetic-sdk-token-'), 'SDK diagnostics must not leak credentials');
    assert.deepEqual(await readdir(privateHome), [], 'no native account/configuration files may be created');
  });
  return {
    directory,
    options,
    launch: async () => {
      try { return daemon = await startDaemon(options); }
      catch (error) { launchRejected = true; throw error; }
    },
    calls: async () => (await readFile(join(directory, 'calls.jsonl'), 'utf8')).trim().split('\n').map(JSON.parse),
  };
}

test('official SDK queries two immutable daemon identities without opening a model session', { timeout: 15000 }, async t => {
  const a = await setup(t, 'A');
  const b = await setup(t, 'B');
  const [first, second] = await Promise.all([a.launch(), b.launch()]);
  a.options.env[tokenVariable] = 'synthetic-sdk-token-B';
  const [accountA, accountB, modelsA, modelsB] = await Promise.all([
    first.accountInfo(), second.accountInfo(), first.getModels(), second.getModels(),
  ]);
  assert.equal(accountA.userId, 'fixture-A');
  assert.equal(accountB.userId, 'fixture-B');
  assert.equal(modelsA[0].value, 'fixture-model-A');
  assert.equal(modelsB[0].value, 'fixture-model-B');
  await first.stop();
  assert.equal(first.state, 'stopped');
  await assert.rejects(first.accountInfo(), /not ready|stopped/i);
  assert.equal((await second.accountInfo()).userId, 'fixture-B');
  for (const fixture of [a, b]) {
    assert.ok((await fixture.calls()).every(call => [
      'daemon.initialize', 'daemon.account_info', 'daemon.get_models', 'daemon.shutdown',
    ].includes(call.method)), 'account validation must not dispatch inference or sessions');
  }
});

test('missing account capability rejects locally before dispatch', { timeout: 10000 }, async t => {
  const fixture = await setup(t, 'A', 'no-account');
  const daemon = await fixture.launch();
  await assert.rejects(daemon.accountInfo(), /daemon_queries_v1/);
  await assert.rejects(daemon.getModels(), /daemon_queries_v1/);
  assert.deepEqual((await fixture.calls()).map(call => call.method), ['daemon.initialize']);
});

test('invalid auth fails initialization and cleans up without fallback or retry', { timeout: 10000 }, async t => {
  const fixture = await setup(t, 'A', 'auth-error');
  await assert.rejects(fixture.launch(), error => error.code === 'AUTH_INVALID' && error.retryable === false);
  assert.deepEqual((await fixture.calls()).map(call => call.method), ['daemon.initialize']);
});

test('expired account query stays an explicit error, never an empty account or another identity', { timeout: 10000 }, async t => {
  const fixture = await setup(t, 'A', 'expired');
  const daemon = await fixture.launch();
  await assert.rejects(daemon.accountInfo(), error => error.code === 'AUTH_EXPIRED' && error.retryable === false);
  assert.deepEqual((await fixture.calls()).map(call => call.method), ['daemon.initialize', 'daemon.account_info']);
});

test('malformed account reply is rejected instead of reporting logged-in', { timeout: 10000 }, async t => {
  const fixture = await setup(t, 'A', 'malformed-account');
  const daemon = await fixture.launch();
  await assert.rejects(daemon.accountInfo(), /account/);
});

test('legacy protocol is refused at initialization, not silently downgraded', { timeout: 10000 }, async t => {
  const fixture = await setup(t, 'A', 'legacy');
  await assert.rejects(fixture.launch(), /multi_session_v2/);
  assert.deepEqual((await fixture.calls()).map(call => call.method), ['daemon.initialize']);
});

test('query cannot override a selected daemon identity', { timeout: 10000 }, async t => {
  const fixture = await setup(t);
  const daemon = await fixture.launch();
  assert.throws(() => query({
    prompt: 'synthetic query that must never be dispatched',
    options: { daemon, auth: accessToken('synthetic-sdk-token-B') },
  }), /auth.*daemon|daemon.*auth/i);
  assert.deepEqual((await fixture.calls()).map(call => call.method), ['daemon.initialize']);
});

test('missing explicit credential refuses before any native process starts', { timeout: 10000 }, async t => {
  const fixture = await setup(t);
  fixture.options.env[tokenVariable] = undefined;
  await assert.rejects(fixture.launch(), /QODER_SDK_CONTRACT_TOKEN/);
  await assert.rejects(readFile(join(fixture.directory, 'started.json')), error => error.code === 'ENOENT');
});
