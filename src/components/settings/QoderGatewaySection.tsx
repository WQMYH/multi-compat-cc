import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { proxyApi } from "@/lib/api/proxy";
import {
  qoderGatewayApi,
  type QoderApiFormat,
  type QoderGatewayProvider,
  type QoderGatewayStatus,
  type QoderPlanModel,
} from "@/lib/api/qoderGateway";
import { authListAccounts, type ManagedAuthAccount } from "@/lib/api/auth";
import { fetchCodexOauthModels } from "@/lib/api/model-fetch";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

export function QoderGatewaySection() {
  const { t } = useTranslation();
  const label = (key: string, fallback: string) =>
    t(`settings.qoderGateway.${key}`, { defaultValue: fallback });
  const [status, setStatus] = useState<QoderGatewayStatus>({
    selectedId: null,
    keyConfigured: false,
    providers: [],
  });
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState("");
  const [id, setId] = useState<string>();
  const [name, setName] = useState("");
  const [baseUrl, setBaseUrl] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [apiFormat, setApiFormat] = useState<QoderApiFormat>("openai_chat");
  const [models, setModels] = useState("");
  const [planAccountId, setPlanAccountId] = useState("");
  const [planAccounts, setPlanAccounts] = useState<ManagedAuthAccount[]>([]);
  const [legacyAccount, setLegacyAccount] = useState(false);
  const [formOpen, setFormOpen] = useState(false);
  const [planModels, setPlanModels] = useState<QoderPlanModel[]>([]);
  const [connection, setConnection] = useState<{
    baseUrl: string;
    key: string;
  }>();

  useEffect(() => {
    let mounted = true;
    authListAccounts("codex_oauth")
      .then((value) => {
        if (mounted) setPlanAccounts(value);
      })
      .catch(() => {
        // A missing managed account never falls back to another login.
      });
    qoderGatewayApi
      .status()
      .then((value) => {
        if (mounted) setStatus(value);
      })
      .catch((e) => {
        if (mounted) setError(String(e));
      })
      .finally(() => {
        if (mounted) setBusy(false);
      });
    return () => {
      mounted = false;
    };
  }, []);

  const run = async (operation: () => Promise<void>) => {
    setBusy(true);
    setError("");
    try {
      await operation();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };
  const edit = (provider: QoderGatewayProvider) => {
    setFormOpen(true);
    setId(provider.id);
    setName(provider.name);
    setBaseUrl(provider.baseUrl);
    setApiFormat(provider.apiFormat);
    setModels(provider.models.join("\n"));
    setApiKey("");
    setPlanAccountId(provider.codexAccountId ?? "");
    setLegacyAccount(!!provider.planAccountId);
    setPlanModels([]);
  };
  const reset = () => {
    setFormOpen(false);
    setLegacyAccount(false);
    setId(undefined);
    setName("");
    setBaseUrl("");
    setApiKey("");
    setModels("");
    setApiFormat("openai_chat");
    setPlanAccountId("");
    setPlanModels([]);
  };
  const start = () =>
    run(async () => {
      const current = await proxyApi.getProxyStatus();
      const server = current.running
        ? current
        : await proxyApi.startProxyServer();
      const key = await qoderGatewayApi.prepareKey();
      const host =
        server.address === "0.0.0.0"
          ? "127.0.0.1"
          : server.address === "::"
            ? "[::1]"
            : server.address.includes(":") && !server.address.startsWith("[")
              ? `[${server.address}]`
              : server.address;
      setConnection({ baseUrl: `http://${host}:${server.port}/qoder/v1`, key });
      setStatus(await qoderGatewayApi.status());
    });

  return (
    <section
      className="rounded-card border border-border bg-surface p-4 space-y-3"
      aria-label="Qoder CN BYOK Gateway"
    >
      <div className="flex items-center justify-between gap-3">
        <h3 className="font-medium">Qoder CN · 模型供应商</h3>
        <Button
          disabled={busy}
          onClick={() => {
            reset();
            setFormOpen(true);
          }}
        >
          添加供应商
        </Button>
      </div>
      <p className="text-sm text-muted-foreground">
        {label(
          "scope",
          "Third-party model access through an isolated Qoder gateway. Does not change Qoder login, Codex defaults or native configuration. Match Qoder's API type to the provider below.",
        )}
      </p>
      {error && <p role="alert">{error}</p>}
      <ul className="space-y-2">
        {status.providers.length === 0 && (
          <li className="rounded-card border border-dashed p-6 text-center text-fg-2">
            尚无供应商。可添加 CC 已管理的 Codex 账号，或第三方 API。
          </li>
        )}
        {status.providers.map((provider) => (
          <li
            key={provider.id}
            className="rounded-card border border-border p-4 flex flex-wrap gap-2 items-center"
          >
            <span>
              {provider.name} · {provider.apiFormat} ·{" "}
              {provider.models.join(", ")}
              {status.selectedId === provider.id
                ? ` · ${label("selected", "Selected")}`
                : ""}
            </span>
            <Button
              size="sm"
              variant="secondary"
              disabled={busy || status.selectedId === provider.id}
              onClick={() =>
                run(async () => {
                  setStatus(await qoderGatewayApi.select(provider.id));
                })
              }
            >
              {label("select", "Select provider")}
            </Button>
            <Button
              size="sm"
              variant="secondary"
              disabled={busy}
              onClick={() => edit(provider)}
            >
              {label("edit", "Edit provider")}
            </Button>
            <Button
              size="sm"
              variant="secondary"
              disabled={busy}
              onClick={() =>
                run(async () => {
                  setStatus(await qoderGatewayApi.remove(provider.id));
                  if (id === provider.id) reset();
                })
              }
            >
              {label("remove", "Remove provider")}
            </Button>
          </li>
        ))}
      </ul>
      <Dialog open={formOpen} onOpenChange={setFormOpen}>
        <DialogContent className="max-h-[85vh] overflow-y-auto">
          <DialogHeader>
            <DialogTitle>
              {id ? "编辑 Qoder 供应商" : "添加 Qoder 供应商"}
            </DialogTitle>
          </DialogHeader>
          <form
            className="space-y-2"
            onSubmit={(event) => {
              event.preventDefault();
              void run(async () => {
                setStatus(
                  await qoderGatewayApi.save({
                    id,
                    name,
                    baseUrl,
                    apiFormat,
                    apiKey:
                      planAccountId || (id && !apiKey) ? undefined : apiKey,
                    ...(planAccountId ? { codexAccountId: planAccountId } : {}),
                    models: models
                      .split(/\r?\n/)
                      .map((value) => value.trim())
                      .filter(Boolean),
                  }),
                );
                reset();
              });
            }}
          >
            <label className="block">
              {label("name", "Provider name")}
              <Input
                value={name}
                onChange={(event) => setName(event.target.value)}
                required
                disabled={busy}
              />
            </label>
            <label className="block">
              {label("credential", "Credential source")}
              <select
                value={planAccountId}
                disabled={busy}
                onChange={(event) => {
                  const account = event.target.value;
                  setPlanAccountId(account);
                  setPlanModels([]);
                  setApiKey("");
                  setModels("");
                  setBaseUrl(
                    account ? "https://chatgpt.com/backend-api/codex" : "",
                  );
                  setApiFormat(account ? "openai_responses" : "openai_chat");
                }}
              >
                <option value="">
                  {label("apiKeySource", "Explicit API key")}
                </option>
                {planAccounts.map((account) => (
                  <option
                    key={account.id}
                    value={account.id}
                    disabled={
                      account.reauth_required || account.requires_reauth
                    }
                  >
                    {account.login} · {account.id.slice(0, 8)}
                  </option>
                ))}
                {planAccountId &&
                  !planAccounts.some(
                    (account) => account.id === planAccountId,
                  ) && (
                    <option value={planAccountId} disabled>
                      {label(
                        "unavailableAccount",
                        "Unavailable CC Codex account",
                      )}{" "}
                      · {planAccountId.slice(0, 8)}
                    </option>
                  )}
              </select>
            </label>
            <Button
              type="button"
              disabled={busy}
              onClick={() =>
                run(async () => {
                  setPlanAccounts(await authListAccounts("codex_oauth"));
                })
              }
            >
              {label("refreshAccounts", "Refresh CC Codex accounts")}
            </Button>
            {planAccountId && (
              <>
                <p className="text-sm text-muted-foreground">
                  {label(
                    "planRequirements",
                    "Use this explicitly selected CC Codex account through the existing account manager. Responses only, store=false, stream=true; never switch to a different account automatically.",
                  )}
                </p>
                <Button
                  type="button"
                  disabled={busy}
                  onClick={() =>
                    run(async () => {
                      const catalog =
                        await fetchCodexOauthModels(planAccountId);
                      setPlanModels(
                        catalog.map((model) => ({
                          id: model.id,
                          displayName: model.id,
                        })),
                      );
                      setModels(catalog.map((model) => model.id).join("\n"));
                    })
                  }
                >
                  {label("loadModels", "Load this account's models")}
                </Button>
                <ul
                  aria-label={label(
                    "availableModels",
                    "Available account models",
                  )}
                >
                  {planModels.map((model) => (
                    <li key={model.id}>
                      {model.displayName} · {model.id}
                    </li>
                  ))}
                </ul>
              </>
            )}
            <label className="block">
              {label("baseUrl", "Upstream Base URL")}
              <Input
                value={baseUrl}
                onChange={(event) => setBaseUrl(event.target.value)}
                required
                disabled={busy}
                readOnly={!!planAccountId}
                placeholder="https://provider.example/v1"
              />
            </label>
            {!planAccountId && (
              <label className="block">
                {label(
                  "apiKey",
                  "Upstream API key (blank keeps the existing key when editing)",
                )}
                <Input
                  type="password"
                  autoComplete="off"
                  value={apiKey}
                  onChange={(event) => setApiKey(event.target.value)}
                  required={!id}
                  disabled={busy}
                />
              </label>
            )}
            <label className="block">
              {label("format", "Provider API")}
              <select
                value={apiFormat}
                disabled={busy || !!planAccountId}
                onChange={(event) =>
                  setApiFormat(event.target.value as QoderApiFormat)
                }
              >
                <option value="openai_chat">Chat Completions API</option>
                <option value="openai_responses">Responses API</option>
              </select>
            </label>
            <label className="block">
              {label("models", "Model IDs (one per line)")}
              <textarea
                className="w-full rounded border p-2"
                value={models}
                onChange={(event) => setModels(event.target.value)}
                required
                disabled={busy}
              />
            </label>
            {legacyAccount && (
              <p role="alert" className="text-danger">
                旧独立账号绑定已保留，不会自动迁移。请取消编辑，重新选择 CC
                Codex 账号建立供应商。
              </p>
            )}
            <Button type="submit" disabled={busy || legacyAccount}>
              {label("save", "Save gateway provider")}
            </Button>
            {id && (
              <Button
                type="button"
                variant="secondary"
                disabled={busy}
                onClick={reset}
              >
                {label("cancel", "Cancel edit")}
              </Button>
            )}
          </form>
          {error && <p role="alert">{error}</p>}
        </DialogContent>
      </Dialog>
      <Button disabled={busy || !status.selectedId} onClick={start}>
        {label("start", "Start gateway and show connection")}
      </Button>
      {connection && (
        <div className="space-y-2">
          <p>
            {label(
              "setup",
              "In Qoder's custom models, use this Base URL and local gateway key, the selected API type, and a listed Model ID. Upstream credentials stay in CC Switch.",
            )}
          </p>
          <label className="block">
            {label("localUrl", "Local gateway Base URL")}
            <Input readOnly value={connection.baseUrl} />
          </label>
          <label className="block">
            {label("localKey", "Local gateway key")}
            <Input type="password" readOnly value={connection.key} />
          </label>
        </div>
      )}
    </section>
  );
}
