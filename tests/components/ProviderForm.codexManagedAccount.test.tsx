import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  ProviderForm,
  type ProviderFormValues,
} from "@/components/providers/forms/ProviderForm";
import { createTestQueryClient } from "../utils/testQueryClient";

const authState = vi.hoisted(() => ({
  codexReauthRequired: false,
}));
const toastMocks = vi.hoisted(() => ({
  error: vi.fn(),
}));
const qoderModelFetch = vi.hoisted(() => vi.fn());

vi.mock("@/lib/api/model-fetch", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/api/model-fetch")>()),
  fetchCodexOauthModels: qoderModelFetch,
}));

vi.mock("sonner", () => ({
  toast: {
    error: toastMocks.error,
    success: vi.fn(),
  },
}));

vi.mock("@/components/providers/forms/CodexOAuthSection", () => ({
  CodexOAuthSection: ({
    onAccountSelect,
    onSelectionConfirmed,
    onSelectionInvalidated,
    allowUnboundSelection = true,
    allowUnboundSelectionWithoutStatus = false,
  }: {
    onAccountSelect?: (accountId: string | null) => void;
    onSelectionConfirmed?: () => void;
    onSelectionInvalidated?: () => void;
    allowUnboundSelection?: boolean;
    allowUnboundSelectionWithoutStatus?: boolean;
  }) => (
    <div>
      <output data-testid="allow-unbound-selection">
        {allowUnboundSelection ? "true" : "false"}
      </output>
      <output data-testid="allow-unbound-without-status">
        {allowUnboundSelectionWithoutStatus ? "true" : "false"}
      </output>
      <button
        type="button"
        onClick={() => {
          onSelectionConfirmed?.();
          onAccountSelect?.("acct-managed");
        }}
      >
        select-managed-account
      </button>
      {allowUnboundSelection && (
        <button
          type="button"
          onClick={() => {
            onSelectionConfirmed?.();
            onAccountSelect?.(null);
          }}
        >
          select-native-login
        </button>
      )}
      <button
        type="button"
        onClick={() => {
          onSelectionInvalidated?.();
          onAccountSelect?.(null);
        }}
      >
        invalidate-selected-account
      </button>
    </div>
  ),
}));

vi.mock("@/components/JsonEditor", () => ({
  default: ({
    value,
    onChange,
  }: {
    value: string;
    onChange: (value: string) => void;
  }) => (
    <textarea
      data-testid="json-editor"
      value={value}
      onChange={(event) => onChange(event.target.value)}
    />
  ),
}));

vi.mock("@/components/providers/forms/hooks", async (importOriginal) => {
  const actual =
    await importOriginal<typeof import("@/components/providers/forms/hooks")>();
  return {
    ...actual,
    useCopilotAuth: () => ({
      isAuthenticated: false,
      isStatusSuccess: true,
      isStatusError: false,
      accounts: [],
    }),
    useCodexOauth: () => ({
      isAuthenticated: true,
      isStatusSuccess: true,
      isStatusError: false,
      defaultAccountId: "acct-managed",
      accounts: [
        {
          id: "acct-managed",
          login: "user@example.com",
          is_default: true,
          reauth_required: authState.codexReauthRequired,
          requires_reauth: false,
        },
      ],
    }),
    useXaiOauth: () => ({
      isAuthenticated: false,
      accounts: [],
    }),
  };
});

function renderCodexForm(onSubmit: (values: ProviderFormValues) => void) {
  const queryClient = createTestQueryClient();
  return render(
    <QueryClientProvider client={queryClient}>
      <ProviderForm
        appId="codex"
        submitLabel="save-provider"
        onSubmit={onSubmit}
        onCancel={vi.fn()}
      />
    </QueryClientProvider>,
  );
}

function renderClaudeCodexForm(onSubmit: (values: ProviderFormValues) => void) {
  const queryClient = createTestQueryClient();
  return render(
    <QueryClientProvider client={queryClient}>
      <ProviderForm
        appId="claude"
        submitLabel="save-provider"
        onSubmit={onSubmit}
        onCancel={vi.fn()}
        initialData={{
          name: "Claude via Codex OAuth",
          category: "third_party",
          settingsConfig: { env: {} },
          meta: { providerType: "codex_oauth" },
        }}
      />
    </QueryClientProvider>,
  );
}

describe("ProviderForm Codex Official managed account", () => {
  beforeEach(() => {
    authState.codexReauthRequired = false;
    toastMocks.error.mockReset();
    qoderModelFetch.mockReset();
  });

  it("fetches Qoder BYOK model choices from the explicitly selected managed account", async () => {
    Element.prototype.scrollIntoView = vi.fn();
    qoderModelFetch.mockResolvedValue([
      { id: "gpt-6.1-sol", ownedBy: "OpenAI" },
    ]);
    const onSubmit = vi.fn();
    render(
      <QueryClientProvider client={createTestQueryClient()}>
        <ProviderForm
          appId="qoder"
          submitLabel="save-provider"
          onSubmit={onSubmit}
          onCancel={vi.fn()}
        />
      </QueryClientProvider>,
    );
    fireEvent.click(screen.getByRole("button", { name: /OpenAI Official/ }));
    const fetchModels = screen.getByRole("button", {
      name: /fetchModels|获取模型列表/,
    });
    fireEvent.click(fetchModels);
    expect(qoderModelFetch).not.toHaveBeenCalled();
    fireEvent.click(
      screen.getByRole("button", { name: "select-managed-account" }),
    );
    fireEvent.click(fetchModels);
    await waitFor(() =>
      expect(qoderModelFetch).toHaveBeenCalledWith("acct-managed"),
    );
    fireEvent.click(
      await screen.findByRole("button", {
        name: /Select model|选择模型|searchModelAriaLabel/,
      }),
    );
    fireEvent.click(await screen.findByRole("option", { name: "gpt-6.1-sol" }));
    expect(screen.getByRole("textbox", { name: "默认模型" })).toHaveValue(
      "gpt-6.1-sol",
    );
    expect(onSubmit).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "save-provider" }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalledOnce());
    expect(
      JSON.parse(onSubmit.mock.calls[0][0].settingsConfig).config,
    ).toContain('model = "gpt-6.1-sol"');
    expect(JSON.parse(onSubmit.mock.calls[0][0].settingsConfig).auth).toEqual(
      {},
    );
  });

  it("discards in-flight Qoder model choices when their managed account is cleared", async () => {
    let resolveModels!: (models: { id: string; ownedBy: string }[]) => void;
    qoderModelFetch.mockReturnValue(
      new Promise((resolve) => {
        resolveModels = resolve;
      }),
    );
    render(
      <QueryClientProvider client={createTestQueryClient()}>
        <ProviderForm
          appId="qoder"
          submitLabel="save-provider"
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
        />
      </QueryClientProvider>,
    );
    fireEvent.click(screen.getByRole("button", { name: /OpenAI Official/ }));
    fireEvent.click(
      screen.getByRole("button", { name: "select-managed-account" }),
    );
    fireEvent.click(
      screen.getByRole("button", { name: /fetchModels|获取模型列表/ }),
    );
    fireEvent.click(
      screen.getByRole("button", { name: "invalidate-selected-account" }),
    );
    resolveModels([{ id: "stale-account-model", ownedBy: "OpenAI" }]);
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: /fetchModels|获取模型列表/ }),
      ).not.toBeDisabled(),
    );
    expect(
      screen.queryByRole("button", {
        name: /Select model|选择模型|searchModelAriaLabel/,
      }),
    ).not.toBeInTheDocument();
  });

  it.each(["qoder", "dsh"] as const)(
    "reuses the existing managed account chooser for %s without another login",
    async (appId) => {
      const onSubmit = vi.fn();
      render(
        <QueryClientProvider client={createTestQueryClient()}>
          <ProviderForm
            appId={appId}
            submitLabel="save-provider"
            onSubmit={onSubmit}
            onCancel={vi.fn()}
          />
        </QueryClientProvider>,
      );
      fireEvent.click(screen.getByRole("button", { name: /OpenAI Official/ }));
      fireEvent.click(screen.getByRole("button", { name: "save-provider" }));
      await waitFor(() =>
        expect(toastMocks.error).toHaveBeenCalledWith(
          appId === "qoder"
            ? "请先选择 Qoder BYOK 默认模型"
            : "请先选择 DSH 默认模型",
        ),
      );
      expect(onSubmit).not.toHaveBeenCalled();
      fireEvent.change(screen.getByRole("textbox", { name: "默认模型" }), {
        target: { value: "gpt-6.1-sol" },
      });
      fireEvent.click(screen.getByRole("button", { name: "save-provider" }));
      await waitFor(() =>
        expect(toastMocks.error).toHaveBeenCalledWith(
          "请先选择 CC 已托管的 Codex 账号",
        ),
      );
      expect(onSubmit).not.toHaveBeenCalled();
      fireEvent.click(
        await screen.findByRole("button", { name: "select-managed-account" }),
      );
      fireEvent.click(screen.getByRole("button", { name: "save-provider" }));
      await waitFor(() => expect(onSubmit).toHaveBeenCalledOnce());
      const value = onSubmit.mock.calls[0][0] as ProviderFormValues;
      expect(value.meta?.authBinding?.accountId).toBe("acct-managed");
      expect(JSON.parse(value.settingsConfig).auth).toEqual({});
      expect(JSON.parse(value.settingsConfig).config).toContain(
        'model = "gpt-6.1-sol"',
      );
    },
  );

  it("does not offer an ambient Codex login for Qoder BYOK", () => {
    render(
      <QueryClientProvider client={createTestQueryClient()}>
        <ProviderForm
          appId="qoder"
          submitLabel="save-provider"
          onSubmit={vi.fn()}
          onCancel={vi.fn()}
        />
      </QueryClientProvider>,
    );
    fireEvent.click(screen.getByRole("button", { name: /OpenAI Official/ }));
    expect(
      screen.queryByRole("button", { name: "select-native-login" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByTestId("allow-unbound-without-status"),
    ).toHaveTextContent("false");
  });

  it.each(["qoder", "dsh"] as const)(
    "repairs an existing %s managed account row with no model through the shared editor",
    async (appId) => {
      const onSubmit = vi.fn();
      render(
        <QueryClientProvider client={createTestQueryClient()}>
          <ProviderForm
            appId={appId}
            providerId={`existing-${appId}`}
            submitLabel="save-provider"
            onSubmit={onSubmit}
            onCancel={vi.fn()}
            initialData={{
              name: `Existing ${appId} account`,
              category: "official",
              settingsConfig: { auth: {}, config: "" },
              meta: {
                providerType: "codex_oauth",
                authBinding: {
                  source: "managed_account",
                  authProvider: "codex_oauth",
                  accountId: "acct-managed",
                },
              },
            }}
          />
        </QueryClientProvider>,
      );
      fireEvent.change(
        await screen.findByRole("textbox", { name: "默认模型" }),
        { target: { value: "gpt-6.1-sol" } },
      );
      fireEvent.click(screen.getByRole("button", { name: "save-provider" }));
      await waitFor(() => expect(onSubmit).toHaveBeenCalledOnce());
      const submitted = onSubmit.mock.calls[0][0] as ProviderFormValues;
      expect(submitted.meta?.authBinding?.accountId).toBe("acct-managed");
      expect(JSON.parse(submitted.settingsConfig).auth).toEqual({});
      expect(JSON.parse(submitted.settingsConfig).config).toContain(
        'model = "gpt-6.1-sol"',
      );
    },
  );

  it("persists the selected managed account while stripping OAuth secrets", async () => {
    const onSubmit = vi.fn();
    renderCodexForm(onSubmit);

    fireEvent.click(screen.getByRole("button", { name: /OpenAI Official/ }));
    fireEvent.click(
      await screen.findByRole("button", { name: "select-managed-account" }),
    );
    fireEvent.click(screen.getByRole("button", { name: "save-provider" }));

    await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
    const submitted = onSubmit.mock.calls[0][0] as ProviderFormValues;
    expect(submitted).toEqual(
      expect.objectContaining({
        name: "OpenAI Official (user@example.com)",
        presetId: "codex-0",
        presetCategory: "official",
        meta: expect.objectContaining({
          providerType: "codex_oauth",
          authBinding: {
            source: "managed_account",
            authProvider: "codex_oauth",
            accountId: "acct-managed",
          },
        }),
      }),
    );
    expect(JSON.parse(submitted.settingsConfig)).toEqual({
      auth: {},
      config: "",
    });
  });

  it("defaults every new Official card to the current Codex login", async () => {
    const onSubmit = vi.fn();
    renderCodexForm(onSubmit);

    fireEvent.click(screen.getByRole("button", { name: /OpenAI Official/ }));
    expect(
      await screen.findByRole("button", { name: "select-native-login" }),
    ).toBeInTheDocument();
    expect(screen.getByTestId("allow-unbound-selection")).toHaveTextContent(
      "true",
    );
    expect(
      screen.getByTestId("allow-unbound-without-status"),
    ).toHaveTextContent("true");
    fireEvent.click(screen.getByRole("button", { name: "save-provider" }));

    await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
    const submitted = onSubmit.mock.calls[0][0] as ProviderFormValues;
    expect(submitted.presetCategory).toBe("official");
    expect(submitted.meta?.providerType).toBeUndefined();
    expect(submitted.meta?.authBinding).toBeUndefined();
    expect(submitted).not.toHaveProperty("codexNativeLoginSelected");
  });

  it("requires confirmation before falling back when a selected account disappears", async () => {
    const onSubmit = vi.fn();
    renderCodexForm(onSubmit);

    fireEvent.click(screen.getByRole("button", { name: /OpenAI Official/ }));
    fireEvent.click(
      await screen.findByRole("button", { name: "select-managed-account" }),
    );
    fireEvent.click(
      screen.getByRole("button", { name: "invalidate-selected-account" }),
    );
    fireEvent.click(screen.getByRole("button", { name: "save-provider" }));

    await waitFor(() =>
      expect(toastMocks.error).toHaveBeenCalledWith("请先选择登录方式"),
    );
    expect(onSubmit).not.toHaveBeenCalled();

    fireEvent.click(
      screen.getByRole("button", { name: "select-native-login" }),
    );
    fireEvent.click(screen.getByRole("button", { name: "save-provider" }));

    await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
    expect(onSubmit.mock.calls[0][0].meta?.providerType).toBeUndefined();
    expect(onSubmit.mock.calls[0][0].meta?.authBinding).toBeUndefined();
  });

  it("allows the fixed Official card to switch to a managed account", async () => {
    const queryClient = createTestQueryClient();
    const onSubmit = vi.fn();
    render(
      <QueryClientProvider client={queryClient}>
        <ProviderForm
          appId="codex"
          providerId="codex-official"
          submitLabel="save-provider"
          onSubmit={onSubmit}
          onCancel={vi.fn()}
          initialData={{
            name: "OpenAI Official",
            settingsConfig: { auth: {}, config: "" },
          }}
        />
      </QueryClientProvider>,
    );

    expect(
      screen.getByRole("button", { name: "select-managed-account" }),
    ).toBeEnabled();
    expect(
      screen.getByRole("button", { name: "select-native-login" }),
    ).toBeInTheDocument();

    fireEvent.click(
      screen.getByRole("button", { name: "select-managed-account" }),
    );
    fireEvent.click(screen.getByRole("button", { name: "save-provider" }));

    await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
    expect(onSubmit.mock.calls[0][0].meta?.authBinding).toEqual({
      source: "managed_account",
      authProvider: "codex_oauth",
      accountId: "acct-managed",
    });
    expect(onSubmit.mock.calls[0][0].meta?.providerType).toBe("codex_oauth");
    expect(onSubmit.mock.calls[0][0]).not.toHaveProperty(
      "codexNativeLoginSelected",
    );
  });

  it.each(["acct-managed", "deleted-account"])(
    "saves the selected account when the fixed card was bound to %s",
    async (previousAccountId) => {
      const queryClient = createTestQueryClient();
      const onSubmit = vi.fn();
      render(
        <QueryClientProvider client={queryClient}>
          <ProviderForm
            appId="codex"
            providerId="codex-official"
            submitLabel="save-provider"
            onSubmit={onSubmit}
            onCancel={vi.fn()}
            initialData={{
              name: "OpenAI Official",
              settingsConfig: { auth: {}, config: "" },
              meta: {
                providerType: "codex_oauth",
                authBinding: {
                  source: "managed_account",
                  authProvider: "codex_oauth",
                  accountId: previousAccountId,
                },
              },
            }}
          />
        </QueryClientProvider>,
      );

      if (previousAccountId === "deleted-account") {
        fireEvent.click(
          screen.getByRole("button", { name: "select-managed-account" }),
        );
      }
      fireEvent.click(screen.getByRole("button", { name: "save-provider" }));
      await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
      expect(onSubmit.mock.calls[0][0].meta?.authBinding).toEqual({
        source: "managed_account",
        authProvider: "codex_oauth",
        accountId: "acct-managed",
      });
    },
  );

  it("keeps a category-less managed card Official when it is unbound", async () => {
    const queryClient = createTestQueryClient();
    const onSubmit = vi.fn();
    render(
      <QueryClientProvider client={queryClient}>
        <ProviderForm
          appId="codex"
          providerId="managed-official"
          submitLabel="save-provider"
          onSubmit={onSubmit}
          onCancel={vi.fn()}
          initialData={{
            name: "OpenAI Official (user@example.com)",
            settingsConfig: { auth: {}, config: "" },
            meta: {
              providerType: "codex_oauth",
              authBinding: {
                source: "managed_account",
                authProvider: "codex_oauth",
                accountId: "acct-managed",
              },
            },
          }}
        />
      </QueryClientProvider>,
    );

    fireEvent.click(
      await screen.findByRole("button", { name: "select-native-login" }),
    );
    fireEvent.click(screen.getByRole("button", { name: "save-provider" }));

    await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
    expect(onSubmit.mock.calls[0][0].meta).not.toEqual(
      expect.objectContaining({ authBinding: expect.anything() }),
    );
    expect(onSubmit.mock.calls[0][0].meta?.providerType).toBeUndefined();
    expect(onSubmit.mock.calls[0][0].presetCategory).toBe("official");
    expect(onSubmit.mock.calls[0][0]).not.toHaveProperty(
      "codexNativeLoginSelected",
    );
  });

  it("keeps an unmarked legacy Official row on follow-login", async () => {
    const queryClient = createTestQueryClient();
    const onSubmit = vi.fn();
    render(
      <QueryClientProvider client={queryClient}>
        <ProviderForm
          appId="codex"
          providerId="legacy-unbound-official"
          submitLabel="save-provider"
          onSubmit={onSubmit}
          onCancel={vi.fn()}
          initialData={{
            name: "Legacy OpenAI Official",
            category: "official",
            settingsConfig: {
              auth: {
                auth_mode: "chatgpt",
                tokens: { refresh_token: "legacy-refresh-token" },
              },
              config: "",
            },
          }}
        />
      </QueryClientProvider>,
    );

    expect(
      screen.getByRole("button", { name: "select-native-login" }),
    ).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "save-provider" }));
    await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
    expect(onSubmit.mock.calls[0][0].meta?.providerType).toBeUndefined();
    expect(onSubmit.mock.calls[0][0].meta?.authBinding).toBeUndefined();
    expect(JSON.parse(onSubmit.mock.calls[0][0].settingsConfig).auth).toEqual({
      auth_mode: "chatgpt",
      tokens: { refresh_token: "legacy-refresh-token" },
    });
  });

  it("does not add OAuth binding behavior to a third-party route with a stale Official category", async () => {
    const queryClient = createTestQueryClient();
    const onSubmit = vi.fn();
    render(
      <QueryClientProvider client={queryClient}>
        <ProviderForm
          appId="codex"
          providerId="legacy-unbound-official"
          submitLabel="save-provider"
          onSubmit={onSubmit}
          onCancel={vi.fn()}
          initialData={{
            name: "Legacy OpenAI Official",
            category: "official",
            settingsConfig: {
              auth: { tokens: { refresh_token: "stale-secret" } },
              config:
                'model_provider = "custom"\nbase_url = "https://example.com/v1"\nexperimental_bearer_token = "stale-key"\n[model_providers.custom]\nrequires_openai_auth = true',
            },
          }}
        />
      </QueryClientProvider>,
    );

    expect(
      screen.queryByRole("button", { name: "select-native-login" }),
    ).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "save-provider" }));

    await waitFor(() => expect(onSubmit).toHaveBeenCalledTimes(1));
    const submitted = onSubmit.mock.calls[0][0] as ProviderFormValues;
    expect(submitted.meta?.providerType).toBeUndefined();
    const submittedSettings = JSON.parse(submitted.settingsConfig);
    expect(submittedSettings.auth).toEqual({
      tokens: { refresh_token: "stale-secret" },
    });
    expect(submittedSettings.config).toContain('model_provider = "custom"');
    expect(submittedSettings.config).toContain(
      'base_url = "https://example.com/v1"',
    );
  });

  it("blocks saving a managed account that requires reauthentication", async () => {
    authState.codexReauthRequired = true;
    const onSubmit = vi.fn();
    renderCodexForm(onSubmit);

    fireEvent.click(screen.getByRole("button", { name: /OpenAI Official/ }));
    fireEvent.click(
      await screen.findByRole("button", { name: "select-managed-account" }),
    );
    fireEvent.click(screen.getByRole("button", { name: "save-provider" }));

    await waitFor(() =>
      expect(toastMocks.error).toHaveBeenCalledWith(
        "已绑定账号不存在或需要重新登录",
      ),
    );
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("blocks the reauth-required default account when no account is selected", async () => {
    authState.codexReauthRequired = true;
    const onSubmit = vi.fn();
    renderClaudeCodexForm(onSubmit);

    fireEvent.click(screen.getByRole("button", { name: "save-provider" }));

    await waitFor(() =>
      expect(toastMocks.error).toHaveBeenCalledWith(
        "已绑定账号不存在或需要重新登录",
      ),
    );
    expect(onSubmit).not.toHaveBeenCalled();
  });
});
