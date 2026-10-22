import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import http from 'node:http';
import { spawn } from 'node:child_process';
import { createRequire } from 'node:module';
import { pathToFileURL } from 'node:url';
import { readNativeDefaults } from '../../src-runtime/dsh-native-defaults.mjs';

const source = fs.readFileSync(new URL('../../src-runtime/dsh-native-defaults.mjs', import.meta.url), 'utf8');
const key = 'synthetic-native-api-secret';
const grant = 'synthetic-native-account-grant';
const balanceUrl = 'https://api.deepseek.com/user/balance';

test('native DSH balances stay with their actual auth owner, return amounts only and never mutate the store', async () => {
  assert.ok(process.env.DSH_TEST_CLI, 'Supply the installed CLI path; native-library acceptance cannot be skipped');
  const home = fs.mkdtempSync(path.join(os.tmpdir(), 'cc-dsh-balance-'));
  const calls = [];
  let expired = false;
  const server = http.createServer((req, res) => {
    calls.push({ url: req.url, authorization: req.headers.authorization, account: req.headers['x-dsh-auth-token'] });
    if (req.url === '/user/balance') {
      res.setHeader('content-type', 'application/json');
      res.end(JSON.stringify({ is_available: true, balance_infos: [{ currency: 'CNY', total_balance: '31.25', key }], secret: key }));
    } else if (req.url === '/api/v0/users/get_user_summary') {
      res.statusCode = expired ? 401 : 200;
      res.setHeader('content-type', 'application/json');
      res.end(JSON.stringify({ code: expired ? 40003 : 0, data: { biz_code: 0, biz_data: {
        normal_wallets: [{ currency: 'CNY', balance: '8.50' }], bonus_wallets: [{ currency: 'CNY', balance: '99' }], secret: grant,
      } } }));
    } else {
      res.statusCode = 500;
      res.end('Unexpected route: no inference, login or other request is allowed');
    }
  });
  await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
  const origin = `http://127.0.0.1:${server.address().port}`;
  const write = (name, value) => {
    const file = path.join(home, name);
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, value, { mode: 0o600 });
  };
  const snapshot = () => Object.fromEntries(fs.readdirSync(home, { recursive: true, withFileTypes: true })
    .filter(entry => entry.isFile()).map(entry => [path.relative(home, path.join(entry.parentPath, entry.name)), fs.readFileSync(path.join(entry.parentPath, entry.name), 'utf8')]));
  const input = { cliPath: process.env.DSH_TEST_CLI, home, balanceUrl };
  const require = createRequire(fs.realpathSync(input.cliPath));
  const boot = await import(pathToFileURL(require.resolve('@deepseek-ai/dsh-app-boot')).href);
  const basePatch = path.join(path.dirname(require.resolve('@deepseek-ai/dsh-base/package.json')), 'cordis.patch.yml');
  const stockAccount = boot.composeEntries([boot.loadOverlayPatches('dsh', basePatch)])
    .find(entry => entry.name === '@deepseek-ai/dsh-deepseek-account-platform');
  assert.ok(stockAccount, 'Exercise actual stock native account configuration, including Loader expressions');
  const credentials = () => JSON.stringify({ version: 1, refs: { NATIVE_API_REF: key }, records: {
    'deepseek-account-platform/default': { kind: 'grant', payload: { version: 1, issuer: origin, token: grant } },
  } });
  const run = providerId => new Promise((resolve, reject) => {
    // Only the API's fixed official endpoint is redirected; platform queries exercise the native provider unchanged.
    const shim = `const realFetch = globalThis.fetch; globalThis.fetch = (url, options) => realFetch(url === ${JSON.stringify(balanceUrl)} ? ${JSON.stringify(origin + '/user/balance')} : url, options);\n`;
    const child = spawn(process.execPath, ['--permission', '--allow-fs-read=*', '--input-type=module', '--eval', shim + source, '--', '--cc-dsh-native-balance'], {
      windowsHide: true, stdio: ['pipe', 'pipe', 'pipe'], env: { ...process.env, NODE_OPTIONS: '', NATIVE_API_REF: 'wrong-cc-ambient-identity' },
    });
    let stdout = '', stderr = '';
    const timer = setTimeout(() => child.kill(), 15_000);
    child.stdout.on('data', data => { stdout += data; });
    child.stderr.on('data', data => { stderr += data; });
    child.on('error', reject);
    child.on('close', code => {
      clearTimeout(timer);
      try {
        for (const secret of [key, grant, 'wrong-cc-ambient-identity']) assert.ok(!(stdout + stderr).includes(secret), 'A native credential must not cross the process result/log boundary');
        resolve({ code, value: JSON.parse(stdout), stderr });
      } catch (error) { reject(error); }
    });
    child.stdin.end(JSON.stringify({ ...input, providerId }));
  });
  try {
    write('profiles/desktop/package.json', JSON.stringify({ dsh: { profile: { bundles: ['@fixture/defaults'] } } }));
    write('profiles/desktop/node_modules/@fixture/defaults/package.json', JSON.stringify({ name: '@fixture/defaults', version: '1.0.0', dsh: { bundle: { patch: 'defaults.yml' } } }));
    write('profiles/desktop/node_modules/@fixture/defaults/defaults.yml', JSON.stringify([{ insert: [
      { id: 'agent-default-model', name: '@deepseek-ai/dsh-agent-default-model', config: { provider: 'deepseek-official', model: 'deepseek-flash' } },
      { id: 'credentials-local', name: '@deepseek-ai/dsh-credentials-local', config: { watch: false } },
      { id: 'llm-deepseek-api-key', name: '@deepseek-ai/dsh-llm-deepseek-api-key', config: { apiKeyEnv: 'NATIVE_API_REF' } },
      { id: 'deepseek-account-platform', name: '@deepseek-ai/dsh-deepseek-account-platform', config: { ...stockAccount.config, platformOrigin: origin, allowLoopbackHttp: true, balanceTimeoutMs: 2_000 } },
    ] }]));
    write('.credentials.yaml', credentials());
    let before = snapshot();
    let [row] = await readNativeDefaults(input);
    assert.equal(row.meta.providerType, 'dsh-native-balance');
    assert.equal(row.meta.usage_script.templateType, 'balance');
    assert.equal(row.settingsConfig.nativeBalanceAuth, 'api-key');
    assert.equal(row.websiteUrl, 'https://platform.deepseek.com');
    assert.ok(!JSON.stringify(row).includes('NATIVE_API_REF'));
    const api = await run(row.id);
    assert.equal(api.code, 0, JSON.stringify(api));
    assert.deepEqual(api.value, { is_available: true, balance_infos: [{ currency: 'CNY', total_balance: '31.25' }] });
    assert.deepEqual(calls, [{ url: '/user/balance', authorization: `Bearer ${key}`, account: undefined }]);
    assert.deepEqual(snapshot(), before);

    write('profiles/desktop/cordis.patch.yml', '- id: agent-default-model\n  config: {provider: deepseek-account, model: deepseek-pro}\n');
    before = snapshot();
    const oldCount = calls.length;
    assert.equal((await run(row.id)).value.error, 'DSH_NATIVE_BALANCE_IDENTITY_CHANGED');
    assert.equal(calls.length, oldCount, 'A stale identity must not dispatch');
    [row] = await readNativeDefaults(input);
    assert.equal(row.settingsConfig.nativeBalanceAuth, 'platform-account');
    const account = await run(row.id);
    assert.equal(account.code, 0, JSON.stringify(account));
    assert.deepEqual(account.value, { is_available: true, balance_infos: [{ currency: 'CNY', total_balance: '8.50' }] });
    assert.deepEqual(calls.at(-1), { url: '/api/v0/users/get_user_summary', authorization: undefined, account: grant });
    assert.deepEqual(snapshot(), before, 'Platform login and API-key storage must stay byte-identical');

    expired = true;
    const denied = await run(row.id);
    assert.notEqual(denied.code, 0);
    assert.deepEqual(snapshot(), before, 'Native expired-grant deletion must be blocked by the read-only child');
    assert.ok(calls.every(call => ['/user/balance', '/api/v0/users/get_user_summary'].includes(call.url)));

    write('profiles/desktop/cordis.patch.yml', '- id: agent-default-model\n  config: {provider: deepseek-official, model: deepseek-flash}\n');
    write('.credentials.yaml', JSON.stringify({ version: 1, refs: {} }));
    before = snapshot();
    [row] = await readNativeDefaults(input);
    const missing = await run(row.id);
    assert.equal(missing.value.error, 'DSH_NATIVE_API_KEY_NOT_CONFIGURED', 'Never substitute the CC environment or platform grant');
    assert.deepEqual(snapshot(), before);

    // A configured native store is the owner even when the application home has another valid key.
    const ownerHome = path.join(home, 'owner-a');
    write('owner-a/.credentials.yaml', credentials());
    write('.credentials.yaml', JSON.stringify({ version: 1, refs: { NATIVE_API_REF: 'wrong-other-native-home' } }));
    write('profiles/desktop/cordis.patch.yml', JSON.stringify([
      { id: 'agent-default-model', config: { provider: 'deepseek-official', model: 'deepseek-flash' } },
      { id: 'credentials-local', config: { dshHome: ownerHome, watch: false } },
    ]));
    before = snapshot();
    [row] = await readNativeDefaults(input);
    const ownStore = await run(row.id);
    assert.equal(ownStore.code, 0, JSON.stringify(ownStore));
    assert.equal(calls.at(-1).authorization, `Bearer ${key}`);
    assert.deepEqual(snapshot(), before);

    write('profiles/desktop/cordis.patch.yml', '- id: agent-default-model\n  config: {provider: deepseek-official, model: deepseek-flash}\n');
    write('.credentials.yaml', 'NATIVE_API_REF: synthetic-legacy\n');
    before = snapshot();
    [row] = await readNativeDefaults(input);
    assert.notEqual((await run(row.id)).code, 0);
    assert.deepEqual(snapshot(), before, 'Native legacy migration is not authorized by a balance query');
  } finally {
    await new Promise(resolve => server.close(resolve));
    assert.equal(path.dirname(home), path.resolve(os.tmpdir()));
    assert.ok(path.basename(home).startsWith('cc-dsh-balance-'));
    fs.rmSync(home, { recursive: true, force: true });
  }
});
