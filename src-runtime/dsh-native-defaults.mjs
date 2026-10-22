import fs from 'node:fs';
import path from 'node:path';
import { createRequire } from 'node:module';
import { pathToFileURL } from 'node:url';
import { createHash } from 'node:crypto';

function installAnchor(cliPath) {
  const require = createRequire(fs.realpathSync(cliPath));
  for (const directory of require.resolve.paths('@deepseek-ai/dsh') ?? []) {
    const file = path.join(directory, '@deepseek-ai/dsh/package.json');
    if (fs.existsSync(file)) return file;
  }
  throw new Error('DSH_NATIVE_CONFIG_LIBRARY_UNAVAILABLE');
}

async function nativeProfiles({ cliPath, home, baseline }) {
  const anchor = installAnchor(cliPath);
  const require = createRequire(anchor);
  const api = await import(pathToFileURL(require.resolve('@deepseek-ai/dsh-app-boot')).href);
  const rows = [];
  for (const profile of ['desktop', 'web']) {
    const dir = path.join(home, 'profiles', profile);
    if (!fs.existsSync(path.join(dir, 'package.json'))) continue;
    const bundles = api.readProfileManifest('dsh', dir).dsh?.profile?.bundles ?? [];
    if (!Array.isArray(bundles) || !bundles.every(name => typeof name === 'string')) throw new Error('DSH_PROFILE_INVALID');
    const exemptions = api.readProfileVersionExemptions(dir);
    const layers = [];
    // Do not call loadProfile[Directory]: those entry points can rewrite manifests.
    for (const name of bundles) {
      try {
        const packageDir = api.resolveBundleDir('dsh', name, anchor, dir);
        const manifest = api.readProfileManifest('dsh', packageDir);
        const issue = api.evaluatePluginCompatibility(manifest, exemptions);
        if (issue && !issue.exempted) continue;
        const patches = api.bundlePatchPaths(packageDir, manifest.dsh.bundle)
          .flatMap(file => api.loadOverlayPatches('dsh', file));
        layers.push({ patches });
      } catch {
        // Native boot also skips missing or incompatible bundle layers.
      }
    }
    const patchPath = path.join(dir, 'cordis.patch.yml');
    const original = baseline?.[profile];
    let ownPatches = api.loadOptionalPatches('dsh', patchPath) ?? [];
    const compose = patches => api.composeEntries([api.readProfilePatches('dsh', {
      dir, installAnchor: anchor, patchPath, home, overlays: [],
    }, { layers, patches })]);
    const currentEntries = compose(ownPatches);
    const currentSelection = currentEntries.find(entry => entry.id === 'agent-default-model'
      || entry.name === '@deepseek-ai/dsh-agent-default-model');
    // The live selection may be CC's route placeholder, not the native default.
    if (original && typeof original === 'object') {
      ownPatches = ownPatches.map(entry => {
        if (entry.id !== 'agent-default-model'
          && entry.name !== '@deepseek-ai/dsh-agent-default-model') return entry;
        const patch = { ...entry };
        delete patch.config;
        return patch;
      });
    }
    const entries = original ? compose(ownPatches) : currentEntries;
    const selection = entries.find(entry => entry.id === 'agent-default-model'
      || entry.name === '@deepseek-ai/dsh-agent-default-model');
    if (!selection || selection.disabled) continue;
    const config = original?.provider && original?.model ? original : selection.config ?? {};
    const { provider, model, reasoningEffort } = config;
    if (provider === 'cc-switch-dsh-route') throw new Error('DSH_NATIVE_DEFAULT_BASELINE_UNAVAILABLE');
    if (typeof provider !== 'string' || !provider.trim() || typeof model !== 'string' || !model.trim()) {
      throw new Error('DSH_DEFAULT_MODEL_UNRESOLVED');
    }
    if (reasoningEffort !== undefined && typeof reasoningEffort !== 'string') throw new Error('DSH_DEFAULT_MODEL_UNRESOLVED');
    const nativeIsCurrent = !currentSelection?.disabled
      && currentSelection?.config?.provider === provider && currentSelection?.config?.model === model;
    rows.push({ profile, dir, entries, provider, model, reasoningEffort, anchor, nativeIsCurrent });
  }
  return { rows, require };
}

function entryFor(entries, name) {
  return entries.find(entry => entry.disabled !== true && entry.name === `@deepseek-ai/dsh-${name}`);
}

async function balanceIdentity(row, require, home) {
  const credentials = entryFor(row.entries, 'credentials-local');
  if (!credentials) return undefined;
  const load = async name => import(pathToFileURL(require.resolve(`@deepseek-ai/dsh-${name}`)).href);
  const { Context } = await import(pathToFileURL(require.resolve('@deepseek-ai/cordis')).href);
  const { interpolate } = await import(pathToFileURL(require.resolve('@deepseek-ai/cordis-plugin-loader')).href);
  const ctx = new Context();
  ctx.provide('profileContext', { name: row.profile, dir: row.dir, home,
    installAnchor: row.anchor, patchPath: path.join(row.dir, 'cordis.patch.yml'), overlays: [] });
  ctx.provide('dshHomePath', home);
  try {
  const configFor = entry => entry && !interpolate(ctx, entry.disabled ?? false) ? interpolate(ctx, entry.config ?? {}) : undefined;
  const credentialConfig = configFor(credentials);
  if (!credentialConfig) return undefined;
  const { resolveSpec } = await load('credentials-local');
  const config = { ...credentialConfig, dshHome: credentialConfig.dshHome ?? home, watch: false };
  // A relative custom store depends on the original Host's working directory, not CC's.
  if ((config.path && !path.isAbsolute(config.path)) || !path.isAbsolute(config.dshHome)) return undefined;
  const file = resolveSpec(config).filename;
  const stat = await fs.promises.stat(file, { bigint: true }).catch(error => {
    if (error.code === 'ENOENT') return undefined;
    throw error;
  });
  let auth, connection, account;
  if (row.provider === 'deepseek-official') {
    const api = entryFor(row.entries, 'llm-deepseek-api-key');
    const apiConfig = configFor(api);
    if (!apiConfig) return undefined;
    const { resolveAdapterOptions } = await load('llm-deepseek-api-key');
    const { createLaunchEnvironmentSnapshot } = await load('launch-environment');
    connection = resolveAdapterOptions(apiConfig, createLaunchEnvironmentSnapshot([]));
    const url = new URL(connection.baseURL);
    if (url.origin !== 'https://api.deepseek.com') return undefined;
    auth = 'api-key';
  } else if (row.provider === 'deepseek-account') {
    const accountConfig = configFor(entryFor(row.entries, 'deepseek-account-platform'));
    if (!accountConfig) return undefined;
    account = { config: accountConfig };
    auth = 'platform-account';
  } else return undefined;
  const revision = createHash('sha256').update(JSON.stringify({
    profile: row.profile, provider: row.provider, model: row.model,
    credentials: config, connection, account: account?.config,
    store: stat ? [String(stat.dev), String(stat.ino), String(stat.size), String(stat.mtimeNs), String(stat.ctimeNs)] : null,
  })).digest('hex').slice(0, 20);
  return { auth, revision, credentials: config, connection, account, load };
  } catch {
    throw new Error('DSH_NATIVE_BALANCE_CONFIG_UNSUPPORTED');
  } finally {
    await ctx.fiber.dispose();
  }
}

export async function readNativeDefaults(input) {
  const { rows, require } = await nativeProfiles(input);
  return Promise.all(rows.map(async row => {
    const balance = await balanceIdentity(row, require, input.home);
    return {
      id: `native-current:${row.profile}${balance ? `:${balance.revision}` : ''}`,
      name: `${row.provider} · ${row.model} · ${row.profile}`,
      category: 'official',
      settingsConfig: { provider: row.provider, model: row.model,
        ...(row.reasoningEffort === undefined ? {} : { reasoningEffort: row.reasoningEffort }), profile: row.profile, nativeIsCurrent: row.nativeIsCurrent,
        ...(balance ? { nativeBalanceAuth: balance.auth } : {}) },
      ...(balance ? { websiteUrl: 'https://platform.deepseek.com', meta: {
        providerType: 'dsh-native-balance',
        usage_script: { enabled: true, language: 'javascript', code: '', templateType: 'balance', autoQueryInterval: 0 },
      } } : {}),
      inFailoverQueue: false,
    };
  }));
}

function balanceBody(wallets, available = true) {
  if (!Array.isArray(wallets) || wallets.length === 0 || wallets.length > 16) throw new Error('DSH_NATIVE_BALANCE_INVALID_RESPONSE');
  return { is_available: available, balance_infos: wallets.map(wallet => {
    const amount = wallet.total_balance ?? wallet.balance;
    if (!['CNY', 'USD'].includes(wallet.currency) || typeof amount !== 'string'
      || !/^-?(?:\d+(?:\.\d*)?|\.\d+)(?:e[+-]?\d+)?$/i.test(amount) || !Number.isFinite(Number(amount))) {
      throw new Error('DSH_NATIVE_BALANCE_INVALID_RESPONSE');
    }
    return { currency: wallet.currency, total_balance: amount };
  }) };
}

export async function readNativeBalance(input) {
  // Native services may migrate a legacy store or revoke an expired grant. Neither is a balance-query write.
  if (!process.permission || process.permission.has('fs.write')) throw new Error('DSH_NATIVE_BALANCE_REQUIRES_READ_ONLY_PROCESS');
  const { rows, require } = await nativeProfiles(input);
  const row = rows.find(row => input.providerId?.startsWith(`native-current:${row.profile}:`));
  if (!row) throw new Error('DSH_NATIVE_BALANCE_PROVIDER_NOT_FOUND');
  const identity = await balanceIdentity(row, require, input.home);
  if (!identity || input.providerId !== `native-current:${row.profile}:${identity.revision}`) throw new Error('DSH_NATIVE_BALANCE_IDENTITY_CHANGED');
  const { Context } = await import(pathToFileURL(require.resolve('@deepseek-ai/cordis')).href);
  const { default: Credentials } = await identity.load('credentials-local');
  const { createLaunchEnvironmentSnapshot } = await identity.load('launch-environment');
  const ctx = new Context();
  // CC's ambient keys are not the DSH Host's identity. Resolve only DSH-owned persisted credentials.
  ctx.provide('launchEnvironment', createLaunchEnvironmentSnapshot([]));
  try {
    try { await ctx.plugin(Credentials, identity.credentials); }
    catch { throw new Error('DSH_NATIVE_CREDENTIALS_UNAVAILABLE'); }
    let body;
    if (identity.auth === 'platform-account') {
      const { default: Authorization } = await identity.load('authorization');
      const { default: Account } = await identity.load('deepseek-account-platform');
      await ctx.plugin(Authorization);
      await ctx.plugin(Account, { ...identity.account.config,
        balanceTimeoutMs: Math.min(identity.account.config?.balanceTimeoutMs ?? 30_000, 15_000) });
      const balance = await ctx.deepseekAccount.getBalance({
        version: JSON.parse(fs.readFileSync(require.resolve('@deepseek-ai/dsh/package.json'), 'utf8')).version,
        locale: 'en-US', timezoneOffsetSeconds: -new Date().getTimezoneOffset() * 60,
      }).catch(error => { throw new Error(error.code === 'ERR_ACCESS_DENIED'
        ? 'DSH_NATIVE_ACCOUNT_AUTH_EXPIRED' : 'DSH_NATIVE_ACCOUNT_BALANCE_FAILED'); });
      if (balance === null) throw new Error('DSH_NATIVE_ACCOUNT_NOT_AUTHORIZED');
      if (balance.status !== 'ready') throw new Error('DSH_NATIVE_ACCOUNT_BALANCE_FAILED');
      body = balanceBody(balance.value);
    } else {
      if (input.balanceUrl !== 'https://api.deepseek.com/user/balance') throw new Error('DSH_NATIVE_BALANCE_ENDPOINT_INVALID');
      const credential = await ctx.credentials.resolve(identity.connection.apiKeyEnv);
      if (!credential?.value) throw new Error('DSH_NATIVE_API_KEY_NOT_CONFIGURED');
      // Transport stays beside the native resolver; CC receives only its existing balance parser's input.
      const response = await fetch(input.balanceUrl, { headers: { Authorization: `Bearer ${credential.value}`, Accept: 'application/json' },
        signal: AbortSignal.timeout(15_000), redirect: 'error' });
      if (!response.ok) throw new Error(response.status === 401 || response.status === 403
        ? 'DSH_NATIVE_BALANCE_UNAUTHORIZED' : response.status >= 500 || response.status === 429
          ? 'DSH_NATIVE_BALANCE_REQUEST_FAILED' : 'DSH_NATIVE_BALANCE_HTTP_REJECTED');
      const value = await response.json();
      body = balanceBody(value.balance_infos, typeof value.is_available === 'boolean' ? value.is_available : true);
    }
    const latestProfile = (await nativeProfiles(input)).rows.find(value => value.profile === row.profile);
    const latest = latestProfile && await balanceIdentity(latestProfile, require, input.home);
    if (latest?.revision !== identity.revision) throw new Error('DSH_NATIVE_BALANCE_IDENTITY_CHANGED');
    return body;
  } finally {
    await ctx.fiber.dispose();
  }
}

if (['--cc-dsh-native-defaults', '--cc-dsh-native-balance'].includes(process.argv[1])) {
  try {
    let input = '';
    for await (const chunk of process.stdin) input += chunk;
    process.stdout.write(JSON.stringify(await (process.argv[1] === '--cc-dsh-native-balance' ? readNativeBalance : readNativeDefaults)(JSON.parse(input))));
  } catch (error) {
    const code = /^DSH_[A-Z_]+$/.test(error.message ?? '') ? error.message
      : process.argv[1] !== '--cc-dsh-native-balance' ? 'DSH_NATIVE_DEFAULTS_READ_FAILED'
      : error.code === 'ERR_ACCESS_DENIED' ? 'DSH_NATIVE_BALANCE_WRITE_BLOCKED' : 'DSH_NATIVE_BALANCE_READ_FAILED';
    process.stdout.write(JSON.stringify({ error: code }));
    process.exitCode = 1;
  }
}
