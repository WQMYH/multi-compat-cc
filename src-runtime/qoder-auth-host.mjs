import { mkdtemp, rm } from "node:fs/promises";
import { isAbsolute, join } from "node:path";
import { pathToFileURL } from "node:url";
import {
  accessTokenFromEnv,
  qodercliAuth,
  startDaemon,
} from "@qodercn-ai/qodercn-agent-sdk";

function invalid(code) {
  const error = new Error(code);
  error.code = code;
  throw error;
}

async function withQoderDaemon(input, action) {
  if (
    !input ||
    typeof input !== "object" ||
    typeof input.cliPath !== "string" ||
    !isAbsolute(input.cliPath)
  )
    invalid("QODER_CLI_PATH_REQUIRED");
  if (typeof input.scratchDir !== "string" || !isAbsolute(input.scratchDir))
    invalid("QODER_SCRATCH_DIRECTORY_REQUIRED");
  const source = input.source;
  if (!source || !["pat_env", "cli"].includes(source.type))
    invalid("QODER_AUTH_SOURCE_REQUIRED");
  const env = { ...process.env };
  let auth;
  if (source.type === "pat_env") {
    if (
      typeof source.envVar !== "string" ||
      !/^[A-Za-z_][A-Za-z0-9_]{0,127}$/.test(source.envVar)
    )
      invalid("QODER_ENV_REFERENCE_INVALID");
    if (!process.env[source.envVar]?.trim())
      invalid("QODER_CREDENTIAL_MISSING");
    auth = accessTokenFromEnv(source.envVar);
  } else {
    if (typeof source.configDir !== "string" || !isAbsolute(source.configDir))
      invalid("QODER_CONFIG_DIRECTORY_REQUIRED");
    env.QODERCN_CONFIG_DIR = source.configDir;
    // An ambient PAT must not override an explicitly chosen native CLI identity.
    env.QODERCN_PERSONAL_ACCESS_TOKEN = undefined;
    env.QODERCN_SERVICE_ACCOUNT_KEY = undefined;
    auth = qodercliAuth();
  }
  // The Rust parent owns this directory, including forced-termination cleanup.
  const diagnostics = await mkdtemp(join(input.scratchDir, "qoder-auth-"));
  for (const key of [
    "QODERCN_PERSONAL_ACCESS_TOKEN",
    "QODERCN_SERVICE_ACCOUNT_KEY",
    "QODER_PERSONAL_ACCESS_TOKEN",
    "QODER_SERVICE_ACCOUNT_KEY",
  ]) {
    if (source.type !== "pat_env" || key !== source.envVar)
      env[key] = undefined;
  }
  if (source.type === "pat_env")
    env.QODERCN_CONFIG_DIR = join(diagnostics, "config");
  let daemon;
  try {
    daemon = await startDaemon({
      auth,
      env,
      pathToQoderCLIExecutable: input.cliPath,
      initializeTimeoutMs: 30000,
      debugFile: join(diagnostics, "sdk.log"),
    });
    return await action(daemon);
  } finally {
    try {
      await daemon?.stop();
    } finally {
      await rm(diagnostics, { recursive: true, force: true });
    }
  }
}

async function verifiedAccount(daemon, input) {
    const account = await daemon.accountInfo();
    if (
      typeof account?.userId !== "string" ||
      !account.userId.trim() ||
      account.userId.length > 256
    )
      invalid("QODER_ACCOUNT_UNVERIFIED");
    if (input.expectedUserId && input.expectedUserId !== account.userId)
      invalid("QODER_ACCOUNT_IDENTITY_CHANGED");
    return account;
}

export async function probeQoderAccount(input) {
  return withQoderDaemon(input, async daemon => {
    const account = await verifiedAccount(daemon, input);
    // Persist display metadata, never the SDK's full response or credentials.
    return Object.fromEntries(
      [
        "userId",
        "name",
        "email",
        "organizationName",
        "subscriptionType",
      ].flatMap((key) =>
        typeof account[key] === "string" && account[key].length <= 256
          ? [[key, account[key]]]
          : [],
      ),
    );
  });
}

export async function manageQoderByok(input) {
  let mutationStarted = false;
  try {
    if (input.source?.type !== "cli" || !["sync", "delete", "reconcile"].includes(input.action) ||
        !/^cc-[a-f0-9]{64}$/.test(input.providerId ?? "")) invalid("QODER_BYOK_INPUT_INVALID");
    const config = input.config;
    const readOnly = input.action === "reconcile";
    if (input.action !== "delete" && (!config || config.providerId !== input.providerId ||
        !["openai", "openai-responses", "anthropic"].includes(config.protocol) ||
        (!readOnly && (typeof config.apiKey !== "string" || !config.apiKey.trim() || config.apiKey.length > 16384 || /[\r\n\0]/.test(config.apiKey))) ||
        (readOnly && Object.hasOwn(config, "apiKey")) ||
        !Array.isArray(config.models) || config.models.length < 1 || config.models.length > 200 ||
        config.models.some(m => typeof m.model !== "string" || !m.model.trim() || m.model.length > 256) ||
        new Set(config.models.map(m => m.model)).size !== config.models.length ||
        !config.models.some(m => m.model === config.model))) invalid("QODER_BYOK_INPUT_INVALID");
    if (input.action !== "delete") {
      let url;
      try { url = new URL(config.baseUrl); } catch { invalid("QODER_BYOK_ENDPOINT_INVALID"); }
      if (!["https:", "http:"].includes(url.protocol) || url.username || url.password || url.search || url.hash)
        invalid("QODER_BYOK_ENDPOINT_INVALID");
    }
    return await withQoderDaemon(input, async daemon => {
      const account = await verifiedAccount(daemon, input);
      const before = (await daemon.listByokConfigs()).filter(p => p.providerId === input.providerId);
      if (before.length > 1) invalid("QODER_BYOK_NATIVE_ID_AMBIGUOUS");
      if (before.length && !input.expectedUserId && !readOnly) invalid("QODER_BYOK_EXISTING_ID_UNBOUND");
      if (input.action === "delete" && before.length === 0)
        return { userId: account.userId, providerId: input.providerId, deleted: true };
      mutationStarted = !readOnly;
      if (input.action === "delete") {
        await daemon.deleteByokConfig({ providerId: input.providerId });
        if ((await daemon.listByokConfigs()).some(p => p.providerId === input.providerId))
          invalid("QODER_BYOK_DELETE_READBACK_FAILED");
        return { userId: account.userId, providerId: input.providerId, deleted: true };
      }
      if (!readOnly && before.length) await daemon.updateByokConfig(config);
      else if (!readOnly) {
        const created = await daemon.createByokConfig(config);
        if (created?.providerId !== input.providerId) invalid("QODER_BYOK_CREATE_ID_MISMATCH");
      }
      const after = readOnly ? before : (await daemon.listByokConfigs()).filter(p => p.providerId === input.providerId);
      if (after.length !== 1 || after[0].defaultModelId !== config.model || after[0].protocol !== config.protocol ||
          after[0].authType !== config.authType || after[0].providerType !== config.type ||
          typeof after[0].baseUrl !== "string" ||
          after[0].baseUrl.replace(/\/$/, "") !== config.baseUrl.replace(/\/$/, "") ||
          !Array.isArray(after[0].models) || after[0].models.length !== config.models.length ||
          !config.models.every(m => after[0].models.some(row => row.id === m.model)))
        invalid("QODER_BYOK_CONFIG_READBACK_FAILED");
      // Account catalogs may omit local BYOK; persisted BYOK IDs remain the directory authority.
      let catalog = [];
      try { catalog = await daemon.getModels({ fetchStrategy: "cache", uid: account.userId }); } catch { /* Selection mapping remains unknown. */ }
      const mapped = config.models.map(model => {
        const matches = catalog.filter(row => row.source === "custom" && row.provider === input.providerId && row.model === model.model);
        if (matches.length !== 1 || typeof matches[0].value !== "string" || !matches[0].value ||
            matches[0].value.length > 2048 || matches[0].isEnabled === false) return null;
        return { model: model.model, value: matches[0].value };
      });
      const modelValues = mapped.every(Boolean) && new Set(mapped.map(m => m.value)).size === mapped.length ? mapped : [];
      return { userId: account.userId, providerId: input.providerId, defaultModelId: config.model,
        modelValues, nativeModelIds: after[0].models.map(model => model.id), catalogSource: "native_byok_configs",
        ...(readOnly ? { readOnlyReconciled: true, historicalRequestConfirmed: false } : {}),
        scope: "byok_provider_default", desktopDefaultChanged: false };
    });
  } catch (error) {
    error.nativeEffect = mutationStarted ? "unknown" : "none";
    throw error;
  }
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(process.argv[1]).href
) {
  try {
    let raw = "";
    for await (const bytes of process.stdin) {
      raw += bytes.toString("utf8");
      if (Buffer.byteLength(raw) > 262144) invalid("QODER_REQUEST_TOO_LARGE");
    }
    const input = JSON.parse(raw);
    const result = input.operation === "byok" ? { byok: await manageQoderByok(input) } : { account: await probeQoderAccount(input) };
    process.stdout.write(`${JSON.stringify({ ok: true, ...result })}\n`);
  } catch (error) {
    const code = error?.code === "auth_required" ? "AUTH_REQUIRED" :
      typeof error?.code === "string" &&
      /^[A-Z][A-Z0-9_]{0,63}$/.test(error.code)
        ? error.code
        : "QODER_ACCOUNT_PROBE_FAILED";
    // SDK/CLI error text may contain credentials; only structured codes leave this process.
    process.stdout.write(
      `${JSON.stringify({ ok: false, code, retryable: false, ...(error.nativeEffect ? { nativeEffect: error.nativeEffect } : {}) })}\n`,
    );
    process.exitCode = 1;
  }
}
