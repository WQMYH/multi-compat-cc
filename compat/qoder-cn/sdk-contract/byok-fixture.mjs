import assert from 'node:assert/strict';
import { appendFileSync, existsSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { createInterface } from 'node:readline';

const root = process.env.QODERCN_CONFIG_DIR;
assert.ok(root);
const mode = process.env.QODER_BYOK_FIXTURE_MODE;
const file = join(root, 'native.json');
const store = () => existsSync(file) ? JSON.parse(readFileSync(file, 'utf8')) : {
  globalModel: 'original-desktop-model', providers: { foreign: { displayName: 'Preserve foreign row' } },
};
const reply = (frame, response, code) => new Promise(resolve => process.stdout.write(JSON.stringify({
  type: 'control_response', response: { request_id: frame.request_id, subtype: code ? 'error' : 'success',
    ...(code ? { code, error: code, retryable: false } : { response }) },
}) + '\n', resolve));
const timer = setTimeout(() => process.exit(3), 30000);
writeFileSync(join(root, 'started.json'), JSON.stringify({ pid: process.pid }));
for await (const line of createInterface({ input: process.stdin })) {
  const frame = JSON.parse(line), method = frame.request?.type;
  assert.equal(frame.type, 'control_request', 'no model prompt is permitted');
  assert.equal(frame.session_id, undefined, 'BYOK operations must not create/attach a session');
  appendFileSync(join(root, 'calls.jsonl'), JSON.stringify({ method, providerId: frame.request.config?.providerId ?? frame.request.target?.providerId }) + '\n');
  if (method === 'daemon.initialize') {
    assert.equal(frame.request.auth.type, 'qodercli');
    assert.equal(process.env.QODERCN_PERSONAL_ACCESS_TOKEN, undefined);
    await reply(frame, { daemon_protocol_version: 2, daemon_version: 'synthetic', capabilities: ['multi_session_v2', 'daemon_queries_v1'] });
  } else if (method === 'daemon.account_info') {
    await reply(frame, { account: { userId: mode === 'changed-account' ? 'different-user' : 'fixture-native-user' } });
  } else if (method === 'daemon.list_byok_configs') {
    await reply(frame, { configs: Object.entries(store().providers).filter(([id]) => id !== 'foreign').map(([providerId, config]) => ({
      providerId, displayName: config.displayName, baseUrl: config.baseUrl, protocol: config.protocol,
      providerType: config.type, authType: config.authType, defaultModelId: config.model,
      models: config.models.map(m => ({ id: m.model, displayName: m.displayName ?? m.model, vision: !!m.capabilities?.vision, reasoning: !!m.capabilities?.thinking, maxInputTokens: m.contextWindow ?? 32768, maxOutputTokens: m.maxOutputTokens ?? 4096 })),
    })) });
  } else if (method === 'daemon.create_byok_config' || method === 'daemon.update_byok_config') {
    if (mode === 'deny') { await reply(frame, null, 'BYOK_ACCESS_DENIED'); continue; }
    const config = frame.request.config, next = store();
    assert.match(config.apiKey, /^synthetic-/);
    assert.equal(!!next.providers[config.providerId], method === 'daemon.update_byok_config');
    next.providers[config.providerId] = config;
    writeFileSync(file, JSON.stringify(next));
    if (mode === 'ack-lost') await reply(frame, null, 'SYNTHETIC_ACK_LOST');
    else await reply(frame, method === 'daemon.create_byok_config' ? { config: { providerId: config.providerId } } : undefined);
  } else if (method === 'daemon.get_models') {
    await reply(frame, { models: mode === 'system-only' ? [{ value: 'auto', modelId: 'auto', displayName: 'Auto', description: '', source: 'system' }] : mode === 'missing-model' ? [] : Object.entries(store().providers).filter(([id]) => id !== 'foreign').flatMap(([id, p]) => p.models.map(m => ({
      value: `native-opaque:${id}:${m.model}`, displayName: m.displayName ?? m.model, description: '',
      source: 'custom', provider: id, model: m.model,
    }))) });
  } else if (method === 'daemon.delete_byok_config') {
    const next = store(); delete next.providers[frame.request.target.providerId]; writeFileSync(file, JSON.stringify(next));
    await reply(frame);
  } else {
    assert.equal(method, 'daemon.shutdown', 'inference/check/login/session calls are forbidden');
    await reply(frame); clearTimeout(timer); break;
  }
}
process.exit(0);
