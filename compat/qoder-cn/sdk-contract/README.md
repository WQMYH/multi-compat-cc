# Qoder CN official SDK account contract probe

This uses the real published `@qodercn-ai/qodercn-agent-sdk` 1.0.53, with its npm integrity pinned in the root lockfile. The version pins this diagnostic baseline, not supported Qoder product versions. The SDK license is governed by Qoder product terms; its package is not vendored or relicensed here.

```sh
pnpm install --frozen-lockfile --ignore-scripts
pnpm test:qoder-cn-sdk
```

The SDK's postinstall downloads a native CLI, so it is explicitly disabled in `pnpm-workspace.yaml`. It is a development dependency only. This probe always passes the local `daemon-fixture.mjs` as the executable; it does not locate or start installed Qoder, log in, read an auth cache, contact an inference endpoint or use real credentials.

The native endpoint is synthetic; the SDK's spawn, authentication preparation, initialize, account/model methods, frame parsing, capability validation, errors and shutdown are real. Each test retains isolated temporary diagnostics; only method/profile metadata is recorded. All launched child processes must be gone, successful shutdown must have a clean exit record, and diagnostics must not contain the dummy credentials. The synthetic native endpoint itself makes no network requests.

## Coverage

- Concurrent A/B daemons return their own account/model identity. Changing the original options after startup does not change the running identity; stopping A does not stop B.
- Account queries do not create a Query, worker session or model prompt.
- Missing `daemon_queries_v1` refuses account/model operations before dispatch.
- Invalid initialization auth and expired account auth remain explicit failures without another account, retry or fallback.
- Malformed account payloads and missing mandatory daemon protocol capability are rejected.
- A daemon-bound Query cannot override its authorization.
- A missing explicit credential refuses before starting a child process.

## Evidence limits

This is SDK/synthetic-transport evidence, **not native Qoder account or inference validation**, and no production CC Switch account panel or gateway is implemented by these tests. A null/malformed reply being rejected does not prove that every empty account snapshot means logged-in; the future account UI must require a positively identified account.

The public SDK supports daemon-bound account, model and BYOK operations. Login/logout/status are documented by its `types/account.d.ts` as CLI operations without SDK typed wrappers. A daemon-bound Query inherits startup authentication; account selection must bind to the corresponding daemon lifecycle, not mutate shared in-flight identity.

The SDK accepts user messages and host MCP tool handlers, but this probe does not establish raw Chat Completions/Responses history, external tool pause/resume or arbitrary client compatibility. Agent delegation and model-provider compatibility remain separate acceptance items. The existing 15-request BYOK fixture is independent and is not rerun by this command.

## CC application integration: BYOK

Qoder uses the same application-level provider cards, preset dialogs, universal-provider children and managed Codex account references as the existing CC flows. Its catalog contains only providers added for Qoder, plus a read-only observation of the native configured model; it does not mirror the Codex provider table.

- Add a Qoder provider through the existing dialog, or enable Qoder on an existing universal provider. `同步 BYOK` applies that row through the official daemon's `listByokConfigs`, `createByokConfig` / `updateByokConfig` and `getModels` methods. Editing or deleting an already-bound row follows the same native lifecycle.
- Chat Completions, Responses and Anthropic configurations use the SDK's native protocols. Managed Codex accounts stay in the existing CC account manager; Qoder receives a provider-pinned local CC gateway route, not a copied OAuth credential or another login flow.
- Native CLI discovery checks official npm package metadata and the installed standalone app's worker layout, without a product-version path. An IDE launcher with the same command name is not accepted as the native SDK executable. Missing or ambiguous installations return to CC application management rather than request manual paths.
- The native account identity, provider-default model and canonical `ModelInfo.value` are checked on readback. Foreign providers and the desktop's global model are preserved. An unknown mutation remains pending and blocks another apply/edit/delete until reconciled; it is not retried automatically.

This implements **BYOK roster management and the default model inside that BYOK provider**, not a persistent desktop-wide model switch. The inspected native `set_model` handler is temporary (`isTemporary: true`, `persisted: false`), and the verified daemon API supplies no desktop-global setter. Use Qoder's native selector for that scope, or `Query.setModel` only inside an SDK-owned session. Reasoning capability metadata is not evidence that a desktop session's reasoning setting changed.

The source8 regressions run the bundled, real SDK against a synthetic native process, including the normal CC save/apply/edit/delete entry points. They cover all three protocols, canonical identifiers, account drift, foreign/global preservation and lost-acknowledgment holds. They do not read real credentials, invoke a model, prove account entitlement or establish real Qoder/DSH runtime acceptance. The separate read-only CC acceptance build deliberately blocks configuration mutations and model requests.
