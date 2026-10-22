import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import * as native from '../../src-runtime/dsh-native-defaults.mjs';

test('native inheritance uses absent config rather than an incomplete replacement config', async () => {
  const { createRequire } = await import('node:module');
  const { pathToFileURL } = await import('node:url');
  const require = createRequire(process.env.DSH_TEST_CLI);
  const { composeEntries } = await import(pathToFileURL(require.resolve('@deepseek-ai/dsh-app-boot')).href);
  const base = [{ insert: [{ id: 'agent-default-model', name: '@deepseek-ai/dsh-agent-default-model', config: { provider: 'deepseek-official', model: 'deepseek-v4-pro' } }] }];
  assert.deepEqual(composeEntries([base, [{ id: 'agent-default-model', config: { other: 'keep' } }]])[0].config, { other: 'keep' });
  assert.deepEqual(composeEntries([base, [{ id: 'agent-default-model' }]])[0].config, { provider: 'deepseek-official', model: 'deepseek-v4-pro' });
});

test('review fixes preserve configured CC identity observations and disabled default status with builtin catalogs', async () => {
  assert.ok(process.env.DSH_TEST_CLI);
  const home = fs.mkdtempSync(path.join(os.tmpdir(), 'cc-dsh-inventory-review-'));
  const dir = path.join(home, 'profiles/desktop');
  const bundle = path.join(dir, 'node_modules/@fixture/catalog');
  fs.mkdirSync(bundle, { recursive: true });
  try {
    fs.writeFileSync(path.join(dir, 'package.json'), JSON.stringify({ dsh: { profile: { bundles: ['@fixture/catalog'] } } }));
    fs.writeFileSync(path.join(bundle, 'package.json'), JSON.stringify({ name: '@fixture/catalog', version: '1.0.0', dsh: { bundle: { patch: 'defaults.yml' } } }));
    fs.writeFileSync(path.join(bundle, 'defaults.yml'), JSON.stringify([{ insert: [
      { id: 'agent-default-model', name: '@deepseek-ai/dsh-agent-default-model', config: { provider: 'cc-fixture', model: 'cc-model' } },
      { id: 'llm-pi-ai', name: '@deepseek-ai/dsh-llm-pi-ai', config: { providers: {
        openai: { api: 'openai-responses', apiKeyEnv: 'SYNTHETIC_ONLY' },
        'cc-fixture': { models: [{ id: 'cc-model' }] },
      } } },
    ] }]));
    const input = { cliPath: process.env.DSH_TEST_CLI, home, excludeProviders: ['cc-fixture'], ccProviders: { 'cc-fixture': 'api-a' } };
    const rows = await native.readNativeProviders(input);
    assert.equal(rows.length, 2);
    const current = rows.find(row => row.id === 'api-a');
    assert.equal(current.settingsConfig.nativeObservation, true);
    assert.equal(current.settingsConfig.nativeIsCurrent, true);
    const openai = rows.find(row => row.settingsConfig.provider === 'openai');
    assert.ok(openai.settingsConfig.modelCatalog.models.length > 1, 'Use the installed builtin catalog for an inherited provider');
    const patch = JSON.stringify([{ id: 'agent-default-model', disabled: true }]);
    fs.writeFileSync(path.join(dir, 'cordis.patch.yml'), patch);
    const disabled = await native.readNativeProviders(input);
    assert.equal(disabled.find(row => row.id === 'api-a').settingsConfig.nativeIsCurrent, false);
    assert.equal(disabled.find(row => row.settingsConfig.provider === 'openai').settingsConfig.nativeSelectionDisabled, true);
    assert.equal(fs.readFileSync(path.join(dir, 'cordis.patch.yml'), 'utf8'), patch);
  } finally {
    assert.equal(path.dirname(home), path.resolve(os.tmpdir()));
    assert.ok(path.basename(home).startsWith('cc-dsh-inventory-review-'));
    fs.rmSync(home, { recursive: true, force: true });
  }
});

test('native inventory lists each configured provider and model without copying credentials or changing selection', async () => {
  assert.ok(process.env.DSH_TEST_CLI, 'Use the installed native configuration library');
  const home = fs.mkdtempSync(path.join(os.tmpdir(), 'cc-dsh-inventory-'));
  const write = (name, value) => {
    const file = path.join(home, name);
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, typeof value === 'string' ? value : JSON.stringify(value));
  };
  const snapshot = () => Object.fromEntries(fs.readdirSync(home, { recursive: true, withFileTypes: true })
    .filter(entry => entry.isFile()).map(entry => {
      const file = path.join(entry.parentPath, entry.name);
      return [file, fs.readFileSync(file, 'utf8')];
    }));
  const entries = [
    { id: 'agent-default-model', name: '@deepseek-ai/dsh-agent-default-model', config: { provider: 'deepseek-official', model: 'deepseek-flash' } },
    { id: 'llm-deepseek', name: '@deepseek-ai/dsh-llm-deepseek-api-key', config: {} },
    { id: 'credentials-local', name: '@deepseek-ai/dsh-credentials-local', config: { dshHome: home, watch: false } },
    { id: 'account-platform', name: '@deepseek-ai/dsh-deepseek-account-platform', config: {} },
    { id: 'llm-account', name: '@deepseek-ai/dsh-llm-deepseek-account', config: {} },
    { id: 'llm-pi-ai', name: '@deepseek-ai/dsh-llm-pi-ai', config: { providers: {
      lab: { displayName: 'Lab', api: 'openai-responses', baseURL: 'https://example.invalid/v1',
        apiKey: 'SENTINEL_NATIVE_SECRET', headers: { Authorization: 'SENTINEL_HEADER' },
        models: [{ id: 'first', name: 'First model' }, { id: 'second' }] },
      'cc-switch-dsh-route': { models: [{ id: 'cc-route' }] },
      'cc-existing': { models: [{ id: 'existing' }] },
    } } },
  ];
  try {
    for (const profile of ['desktop', 'web']) {
      write(`profiles/${profile}/package.json`, { dsh: { profile: { bundles: ['@fixture/catalog'] } } });
      write(`profiles/${profile}/node_modules/@fixture/catalog/package.json`, {
        name: '@fixture/catalog', version: '1.0.0', dsh: { bundle: { patch: 'defaults.yml' } },
      });
      write(`profiles/${profile}/node_modules/@fixture/catalog/defaults.yml`, [{ insert: entries }]);
    }
    const refs = { ORIGINAL: 'SENTINEL_STORE_SECRET', DEEPSEEK_API_KEY: 'SENTINEL_API_KEY' };
    write('.credentials.yaml', { version: 1, refs });
    const input = { cliPath: process.env.DSH_TEST_CLI, home, excludeProviders: ['cc-existing'] };
    let before = snapshot();
    const rows = await native.readNativeProviders(input);
    assert.equal(rows.length, 4, 'One official and one configured custom provider per profile; no CC projection mirrors');
    assert.ok(rows.every(row => row.settingsConfig.provider !== 'deepseek-account'),
      'An installed account adapter without a stored grant is not an added account');
    const desktop = rows.find(row => row.settingsConfig.profile === 'desktop' && row.settingsConfig.provider === 'lab');
    assert.ok(desktop.id.startsWith('native-provider:desktop:'));
    assert.deepEqual(desktop.settingsConfig.modelCatalog.models.map(model => model.model), ['first', 'second']);
    assert.equal(desktop.settingsConfig.nativeIsCurrent, false);
    assert.ok(rows.find(row => row.settingsConfig.provider === 'deepseek-official').settingsConfig.modelCatalog.models.length > 1,
      'Use the installed official model catalog, not only the selected model');
    for (const secret of ['SENTINEL_NATIVE_SECRET', 'SENTINEL_HEADER', 'SENTINEL_STORE_SECRET', 'SENTINEL_API_KEY']) {
      assert.ok(!JSON.stringify(rows).includes(secret), 'Native secret must not enter the inventory result');
    }
    assert.deepEqual(snapshot(), before);

    write('profiles/desktop/cordis.patch.yml', [{ id: 'agent-default-model', config: { provider: 'lab', model: 'second' } }]);
    before = snapshot();
    const changed = await native.readNativeProviders(input);
    const selected = changed.find(row => row.id === desktop.id);
    assert.equal(selected.settingsConfig.nativeIsCurrent, true);
    assert.equal(selected.settingsConfig.model, 'second');
    assert.equal(changed.find(row => row.settingsConfig.profile === 'web' && row.settingsConfig.provider === 'deepseek-official').settingsConfig.nativeIsCurrent, true);
    assert.deepEqual(snapshot(), before);

    write('.credentials.yaml', { version: 1, refs, records: {
      'deepseek-account-platform/default': { kind: 'grant', payload: { synthetic: 'SENTINEL_ACCOUNT_GRANT' } },
    } });
    before = snapshot();
    const observed = spawnSync(process.execPath, ['--permission', '--allow-fs-read=*', '--input-type=module',
      '--eval', fs.readFileSync(new URL('../../src-runtime/dsh-native-defaults.mjs', import.meta.url), 'utf8'),
      '--', '--cc-dsh-native-providers'], { input: JSON.stringify(input), encoding: 'utf8', timeout: 10_000 });
    assert.equal(observed.status, 0, observed.stdout);
    const withAccount = JSON.parse(observed.stdout);
    const accounts = withAccount.filter(row => row.settingsConfig.provider === 'deepseek-account');
    assert.deepEqual(accounts.map(row => row.settingsConfig.profile), ['desktop', 'web']);
    assert.ok(accounts.every(row => !row.settingsConfig.nativeIsCurrent), 'Keep configured accounts even when another provider is selected');
    assert.ok(!JSON.stringify(withAccount).includes('SENTINEL_ACCOUNT_GRANT'));
    assert.deepEqual(snapshot(), before, 'Inventory must not initialize account authorization, migrate or rewrite the store');

    write('.credentials.yaml', { version: 1, refs: { ORIGINAL: refs.ORIGINAL } });
    before = snapshot();
    const withoutKey = await native.readNativeProviders(input);
    assert.deepEqual(withoutKey.filter(row => row.settingsConfig.provider === 'deepseek-official')
      .map(row => row.settingsConfig.profile), ['web'], 'Keep the explicit Web default, but do not invent an unconfigured Desktop API account');
    assert.deepEqual(snapshot(), before);

    write('.credentials.yaml', { version: 1, refs });
    const customApi = { id: 'llm-deepseek', config: { baseURL: 'https://native.example.invalid/v1', apiKeyEnv: 'DEEPSEEK_API_KEY' } };
    write('profiles/desktop/cordis.patch.yml', [
      { id: 'agent-default-model', config: { provider: 'lab', model: 'second' } }, customApi,
    ]);
    before = snapshot();
    const customEndpoint = await native.readNativeProviders(input);
    assert.ok(customEndpoint.some(row => row.settingsConfig.profile === 'desktop' && row.settingsConfig.provider === 'deepseek-official'),
      'Native credential presence must not apply the official balance endpoint restriction');
    assert.deepEqual(snapshot(), before);

    write('profiles/desktop/cordis.patch.yml', [customApi]);
    before = snapshot();
    const customDefault = (await native.readNativeDefaults(input)).find(row => row.settingsConfig.profile === 'desktop');
    assert.equal(customDefault.settingsConfig.nativeBalanceAuth, undefined, 'Non-official endpoints must still be excluded from official balance queries');
    assert.equal(customDefault.meta, undefined);
    assert.deepEqual(snapshot(), before);
  } finally {
    assert.equal(path.dirname(home), path.resolve(os.tmpdir()));
    assert.ok(path.basename(home).startsWith('cc-dsh-inventory-'));
    fs.rmSync(home, { recursive: true, force: true });
  }
});
