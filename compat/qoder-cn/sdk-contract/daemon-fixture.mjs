import assert from 'node:assert/strict';
import { appendFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { createInterface } from 'node:readline';

// Synthetic native endpoint only: no SDK mock, network, account cache or model.
const directory = process.env.QODER_SDK_FIXTURE_DIRECTORY;
const profile = process.env.QODER_SDK_FIXTURE_PROFILE;
const mode = process.env.QODER_SDK_FIXTURE_MODE;
assert.ok(directory && ['A', 'B'].includes(profile));
assert.ok(['ok', 'no-account', 'auth-error', 'expired', 'malformed-account', 'legacy'].includes(mode));
assert.deepEqual(process.argv.slice(2), ['--sdk']);
assert.equal(process.env.QODER_AGENT_SDK_DAEMON, '1');
writeFileSync(join(directory, 'started.json'), JSON.stringify({ pid: process.pid, profile }));
process.once('exit', code => {
  writeFileSync(join(directory, 'stopped.json'), JSON.stringify({ code }));
});

const record = event => appendFileSync(join(directory, 'calls.jsonl'), `${JSON.stringify(event)}\n`);
const send = frame => new Promise((resolve, reject) => {
  process.stdout.write(`${JSON.stringify(frame)}\n`, error => error ? reject(error) : resolve());
});
const success = (request, response) => send({
  type: 'control_response',
  response: { subtype: 'success', request_id: request.request_id, ...(response === undefined ? {} : { response }) },
});
const failure = (request, code) => send({
  type: 'control_response',
  response: { subtype: 'error', request_id: request.request_id, error: code, code, retryable: false },
});

let initialized = false;
for await (const line of createInterface({ input: process.stdin })) {
  assert.ok(Buffer.byteLength(line) < 64 * 1024);
  const frame = JSON.parse(line);
  const method = frame.request?.type;
  record({ method, profile });
  assert.equal(frame.type, 'control_request', 'no user/model input is permitted');
  assert.equal(frame.session_id, undefined, 'account queries must not be session-scoped');
  if (method === 'daemon.initialize') {
    assert.equal(initialized, false);
    assert.equal(frame.request.daemon_protocol_version, 2);
    assert.equal(frame.request.auth?.type, 'accessToken');
    assert.equal(frame.request.auth?.accessToken, `synthetic-sdk-token-${profile}`);
    if (mode === 'auth-error') {
      await failure(frame, 'AUTH_INVALID');
      continue;
    }
    initialized = true;
    const capabilities = mode === 'legacy' ? [] : ['multi_session_v2'];
    if (mode !== 'no-account') capabilities.push('daemon_queries_v1');
    await success(frame, { daemon_protocol_version: 2, daemon_version: 'synthetic', capabilities });
  } else if (method === 'daemon.shutdown') {
    await success(frame);
    break;
  } else {
    assert.ok(initialized);
    if (mode === 'expired') {
      await failure(frame, 'AUTH_EXPIRED');
    } else if (method === 'daemon.account_info') {
      await success(frame, { account: mode === 'malformed-account' ? null : { userId: `fixture-${profile}`, name: `Fixture ${profile}` } });
    } else if (method === 'daemon.get_models') {
      await success(frame, { models: [{ value: `fixture-model-${profile}`, displayName: `Fixture ${profile}`, description: 'Synthetic only' }] });
    } else {
      await failure(frame, 'UNSUPPORTED_FIXTURE_OPERATION');
    }
  }
}
process.exit(0);
