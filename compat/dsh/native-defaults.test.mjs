import { test } from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { readNativeDefaults } from '../../src-runtime/dsh-native-defaults.mjs';

test('native DSH layers provide inherited defaults and home overrides without boot or profile writes', async () => {
  assert.ok(process.env.DSH_TEST_CLI, 'Supply the installed CLI path; do not skip native-library acceptance');
  const home = fs.mkdtempSync(path.join(os.tmpdir(), 'cc-dsh-defaults-'));
  const write = (name, value) => { const file = path.join(home, name); fs.mkdirSync(path.dirname(file), { recursive: true }); fs.writeFileSync(file, value); };
  const snapshot = () => Object.fromEntries(fs.readdirSync(home, { recursive: true, withFileTypes: true })
    .filter(entry => entry.isFile()).map(entry => { const file = path.join(entry.parentPath, entry.name); return [file, fs.readFileSync(file, 'utf8')]; }));
  try {
    write('profiles/desktop/package.json', JSON.stringify({ dsh: { profile: { bundles: ['@fixture/defaults', '@deepseek-ai/dsh-experimental-schedule-bundle'] } } }));
    write('profiles/desktop/node_modules/@fixture/defaults/package.json', JSON.stringify({ name: '@fixture/defaults', version: '1.0.0', dsh: { bundle: { patch: 'defaults.yml' } } }));
    write('profiles/desktop/node_modules/@fixture/defaults/defaults.yml', '- insert:\n  - id: agent-default-model\n    name: "@deepseek-ai/dsh-agent-default-model"\n    config: {provider: deepseek-official, model: deepseek-flash}\n  - id: unrelated\n    name: unrelated\n    config: {}\n');
    write('.credentials.yaml', 'refs: {SENTINEL: must-not-be-returned}\n');
    write('cordis.patch.yml', '- id: unrelated\n  config: {value: unrelated-home-overlay}\n');
    const input = { cliPath: process.env.DSH_TEST_CLI, home };
    let before = snapshot();
    const inherited = await readNativeDefaults(input);
    assert.equal(inherited.length, 1);
    assert.deepEqual(inherited[0].settingsConfig, { provider: 'deepseek-official', model: 'deepseek-flash', profile: 'desktop', nativeIsCurrent: true });
    assert.deepEqual(snapshot(), before);
    write('cordis.patch.yml', '[]\n');
    write('profiles/desktop/cordis.patch.yml', '- id: agent-default-model\n  config: {provider: cc-switch-dsh-route, model: route-a}\n');
    const routed = { ...input, baseline: { desktop: { provider: 'original-provider', model: 'original-model' } } };
    const original = await readNativeDefaults(routed);
    assert.deepEqual(original[0].settingsConfig, { provider: 'original-provider', model: 'original-model', profile: 'desktop', nativeIsCurrent: false });
    write('profiles/desktop/cordis.patch.yml', '- id: agent-default-model\n  config: {provider: cc-switch-dsh-route, model: route-b}\n');
    assert.deepEqual(await readNativeDefaults(routed), original);
    assert.equal((await readNativeDefaults({ ...input, baseline: { desktop: {} } }))[0].settingsConfig.model, 'deepseek-flash');
    await assert.rejects(readNativeDefaults(input), /DSH_NATIVE_DEFAULT_BASELINE_UNAVAILABLE/);
    write('profiles/desktop/cordis.patch.yml', '- id: agent-default-model\n  disabled: true\n  config: {provider: cc-switch-dsh-route, model: route-b}\n');
    before = snapshot();
    assert.deepEqual(await readNativeDefaults(routed), [], 'takeover must not re-enable a disabled native default');
    assert.deepEqual(await readNativeDefaults({ ...input, baseline: { desktop: {} } }), []);
    assert.deepEqual(snapshot(), before);
    fs.unlinkSync(path.join(home, 'profiles/desktop/cordis.patch.yml'));
    write('cordis.patch.yml', '- id: unrelated\n  config: {value: unrelated-home-overlay}\n');
    before = snapshot();
    assert.ok(!JSON.stringify(inherited).includes('SENTINEL'));
    const source = fs.readFileSync(new URL('../../src-runtime/dsh-native-defaults.mjs', import.meta.url), 'utf8');
    const invoke = () => spawnSync(process.execPath,
      ['--input-type=module', '--eval', source, '--', '--cc-dsh-native-defaults'],
      { input: JSON.stringify(input), encoding: 'utf8', timeout: 10_000, windowsHide: true,
        env: { ...process.env, NODE_OPTIONS: '' } });
    const result = invoke();
    assert.equal(result.status, 0, result.stderr);
    assert.deepEqual(JSON.parse(result.stdout), inherited);
    assert.deepEqual(snapshot(), before);
    write('profiles/desktop/cordis.patch.yml', '- id: agent-default-model\n  config: {provider: profile-provider, model: profile-model}\n');
    write('cordis.patch.yml', '- id: agent-default-model\n  config: {provider: home-provider, model: home-model}\n');
    before = snapshot();
    const overridden = await readNativeDefaults(input);
    assert.deepEqual(overridden[0].settingsConfig, { provider: 'home-provider', model: 'home-model', profile: 'desktop', nativeIsCurrent: true });
    assert.deepEqual(snapshot(), before);
    write('cordis.patch.yml', '[]\n');
    write('profiles/desktop/node_modules/@fixture/defaults/defaults.yml', '- insert:\n  - id: agent-default-model\n    name: "@deepseek-ai/dsh-agent-default-model"\n    config: {provider: deepseek-official, model: deepseek-flash, reasoningEffort: low}\n');
    write('profiles/desktop/cordis.patch.yml', '- id: agent-default-model\n  config: {provider: cc-legacy, model: old-model, reasoningEffort: high, other: keep}\n');
    before = snapshot();
    const legacy = { ...input, baseline: { desktop: {} } };
    const restored = await readNativeDefaults(legacy);
    assert.deepEqual(restored[0].settingsConfig, { provider: 'deepseek-official', model: 'deepseek-flash', reasoningEffort: 'low', profile: 'desktop', nativeIsCurrent: false });
    assert.deepEqual(snapshot(), before);
    write('cordis.patch.yml', '- id: agent-default-model\n  config: {provider: home-provider, model: home-model, reasoningEffort: minimal}\n');
    before = snapshot();
    assert.deepEqual((await readNativeDefaults(legacy))[0].settingsConfig, { provider: 'home-provider', model: 'home-model', reasoningEffort: 'minimal', profile: 'desktop', nativeIsCurrent: true });
    assert.deepEqual(snapshot(), before);
    write('cordis.patch.yml', '- id: agent-default-model\n  config: {provider: !!js "throw new Error(\'must-not-evaluate\')", model: dynamic}\n');
    await assert.rejects(readNativeDefaults(input), /DSH_DEFAULT_MODEL_UNRESOLVED/);
    const unresolved = invoke();
    assert.equal(unresolved.status, 1);
    assert.deepEqual(JSON.parse(unresolved.stdout), { error: 'DSH_DEFAULT_MODEL_UNRESOLVED' });
  } finally {
    assert.equal(path.dirname(home), path.resolve(os.tmpdir()));
    assert.ok(path.basename(home).startsWith('cc-dsh-defaults-'));
    fs.rmSync(home, { recursive: true, force: true });
  }
});
