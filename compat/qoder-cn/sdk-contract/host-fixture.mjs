import assert from 'node:assert/strict';
import { existsSync, readdirSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createInterface } from 'node:readline';

const directory = process.env.QODER_HOST_FIXTURE_DIRECTORY || process.env.QODERCN_CONFIG_DIR;
assert.ok(directory);
writeFileSync(join(directory, 'child.json'), JSON.stringify({
  pid: process.pid, temporaryDirectory: tmpdir(), configDir: process.env.QODERCN_CONFIG_DIR,
  temporaryFiles: readdirSync(tmpdir()),
}));
const watchdog = setTimeout(() => process.exit(2), 30000);
const send = async (frame, response) => new Promise(resolve => process.stdout.write(`${JSON.stringify({
  type: 'control_response', response: { subtype: 'success', request_id: frame.request_id, ...(response === undefined ? {} : { response }) },
})}\n`, resolve));
for await (const line of createInterface({ input: process.stdin })) {
  const frame = JSON.parse(line);
  if (frame.request?.type === 'daemon.initialize') {
    if (process.env.QODER_HOST_FIXTURE_AUTH_REQUIRED === '1') {
      process.stdout.write(`${JSON.stringify({ type: 'control_response', response: {
        subtype: 'error', request_id: frame.request_id, error: 'Login required',
        code: 'auth_required', retryable: false,
      } })}\n`);
      continue;
    }
    assert.ok(frame.request.auth?.type === 'accessToken' || frame.request.auth?.type === 'qodercli');
    if (frame.request.auth.type === 'accessToken') {
      assert.equal(frame.request.auth.accessToken, 'synthetic-host-token');
      assert.equal(process.env.QODERCN_PERSONAL_ACCESS_TOKEN, undefined);
    }
    else {
      assert.equal(process.env.QODERCN_PERSONAL_ACCESS_TOKEN, undefined);
      assert.equal(process.env.QODERCN_CONFIG_DIR, directory);
    }
    await send(frame, { daemon_protocol_version: 2, daemon_version: 'fixture', capabilities: ['multi_session_v2', 'daemon_queries_v1'] });
  } else if (frame.request?.type === 'daemon.account_info') {
    if (existsSync(join(directory, 'hold-account'))) {
      writeFileSync(join(directory, 'ready.json'), JSON.stringify({ pid: process.pid }));
      continue;
    }
    await send(frame, { account: process.env.QODER_HOST_FIXTURE_EMPTY === '1' ? {} : { userId: 'fixture-A', name: 'Fixture A', privateField: 'must-not-escape' } });
  } else {
    assert.equal(frame.request?.type, 'daemon.shutdown', 'no inference or other native actions permitted');
    await send(frame);
    clearTimeout(watchdog);
    process.exit(0);
  }
}
