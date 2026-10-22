import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdir, writeFile, appendFile } from "node:fs/promises";
import { resolve, join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { executeDsh } from "../../src-runtime/dsh-native-host.mjs";

// Explicit opt-in integration check: an isolated official CLI, no inference or real credentials.
const [cliArg, fixtureArg] = process.argv.slice(2);
assert(cliArg && fixtureArg, "Pass the installed dsh executable and a fresh fixture directory");
const cli = resolve(cliArg), fixture = resolve(fixtureArg);
assert(!/["%\r\n!&|<>]/.test(cli), "Fixture executable must have a literal shell-safe path");
await mkdir(fixture, { recursive: false });
const workspace = join(fixture, "workspace");
await mkdir(workspace);
const env = Object.fromEntries(Object.entries(process.env).filter(([key]) =>
  !/(?:KEY|TOKEN|SECRET|PASSWORD|NODE_OPTIONS)$/i.test(key) && !/^DSH_/i.test(key)));
Object.assign(env, { DSH_HOME: join(fixture, "home"), NO_PROXY: "*", NODE_USE_ENV_PROXY: "0" });
env.CC_DSH_FIXTURE_CLI = cli;
const native = process.platform === "win32"
  ? spawn("powershell.exe", ["-NoProfile", "-NonInteractive", "-Command", "& $env:CC_DSH_FIXTURE_CLI web --host 127.0.0.1 --port 0 --no-open"],
      { cwd: workspace, env, windowsHide: true, stdio: ["ignore", "pipe", "pipe"] })
  : spawn(cli, ["web", "--host", "127.0.0.1", "--port", "0", "--no-open"],
      { cwd: workspace, env, stdio: ["ignore", "pipe", "pipe"] });
let output = "", launchUrl;
const capture = chunk => {
  output = (output + chunk.toString()).slice(-256000);
  launchUrl ??= output.match(/https?:\/\/(?:127\.0\.0\.1|localhost):\d+\/\?token=[A-Za-z0-9._~%+-]+/)?.[0];
};
native.stdout.on("data", capture);
native.stderr.on("data", capture);
let spawnError;
native.on("error", error => { spawnError = error.code; });
const report = { startedAt: new Date().toISOString(), calls: [], inference: "not_run", realCredentials: "not_used" };
const invoke = async (operation, args) => {
  await appendFile(join(fixture, "intent.jsonl"), JSON.stringify({ at: new Date().toISOString(), operation,
    id: args?.id, revision: args?.revision }) + "\n");
  const reply = await executeDsh({ launchUrl, operation, ...(args ? { args } : {}) });
  report.calls.push({ operation, ...reply });
  await writeFile(join(fixture, "result.json"), JSON.stringify(report, null, 2));
  return reply;
};
try {
  for (let i = 0; i < 120 && !launchUrl && native.exitCode === null && !spawnError; i++) await delay(500);
  assert(launchUrl, `DSH_NATIVE_START_FAILED:${spawnError ?? native.exitCode ?? "timeout"}`);
  const inspected = await invoke("inspect");
  assert.equal(inspected.ok, true, inspected.code);
  report.before = inspected.value;
  const id = "cc-switch-isolated-fixture";
  assert(!inspected.value.providers.some(provider => provider.id === id));
  const created = await invoke("save_provider", { mode: "create", revision: inspected.value.revision, id,
    displayName: "Isolated fixture", baseURL: "https://example.invalid/v1", api: "openai-completions",
    models: ["fixture-no-inference"], apiKey: "NOT_A_REAL_API_KEY" });
  assert.equal(created.ok, true, created.code);
  assert.equal(created.providerState, "applied");
  assert.equal(created.credentialState, "applied");
  const readback = await invoke("inspect");
  assert.equal(readback.ok, true, readback.code);
  assert.equal(readback.value.providers.find(provider => provider.id === id)?.models[0], "fixture-no-inference");
  const stale = await invoke("remove_provider", { revision: inspected.value.revision, id });
  assert.equal(stale.code, "settings/conflict");
  assert(!stale.calls.some(call => call.endpoint === "settings/mutate"));
  const removed = await invoke("remove_provider", { revision: readback.value.revision, id });
  assert.equal(removed.ok, true, removed.code);
  const final = await invoke("inspect");
  assert.equal(final.ok, true, final.code);
  assert.deepEqual(final.value.providers, inspected.value.providers);
  report.outcome = "pass_native_configuration";
} catch (error) {
  report.outcome = "failed";
  report.error = String(error.message).replace(/token=[^\s"']+/g, "token=REDACTED");
  process.exitCode = 1;
} finally {
  if (native.exitCode === null && !spawnError) {
    if (process.platform === "win32") await new Promise(resolve => {
      const stop = spawn("taskkill.exe", ["/pid", String(native.pid), "/t", "/f"], { windowsHide: true, stdio: "ignore" });
      stop.on("exit", resolve); stop.on("error", resolve);
    });
    else native.kill("SIGTERM");
    if (native.exitCode === null && native.signalCode === null) await Promise.race([
      new Promise(resolve => native.once("exit", resolve)), delay(5000),
    ]);
  }
  await writeFile(join(fixture, "native-redacted.log"), output.replace(/token=[^\s"'\x1b]+/g, "token=REDACTED"));
  report.finishedAt = new Date().toISOString();
  report.ownedLauncherStopped = native.exitCode !== null || native.signalCode !== null || Boolean(spawnError);
  await writeFile(join(fixture, "result.json"), JSON.stringify(report, null, 2));
  console.log(JSON.stringify({ outcome: report.outcome, error: report.error, calls: report.calls.length,
    nativeExitCode: native.exitCode, inference: report.inference }));
}
