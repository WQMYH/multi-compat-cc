import { QueryClientProvider } from "@tanstack/react-query";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { http, HttpResponse } from "msw";
import { ProviderCard } from "@/components/providers/ProviderCard";
import type { TemplateType } from "@/config/constants";
import { usageKeys } from "@/lib/query/usage";
import type { Provider } from "@/types";
import { createTestQueryClient } from "../utils/testQueryClient";
import { server } from "../msw/server";

vi.mock("@/components/providers/ProviderActions", () => ({
  ProviderActions: () => null,
}));
vi.mock("@/components/ProviderIcon", () => ({ ProviderIcon: () => null }));
vi.mock("@/components/UsageFooter", () => ({
  default: ({ inline }: { inline: boolean }) =>
    inline ? null : <div>expanded-plan-details</div>,
}));
vi.mock("@/components/SubscriptionQuotaFooter", () => ({
  default: () => <div>official-subscription-quota</div>,
}));
vi.mock("@/components/CodexOauthQuotaFooter", () => ({
  default: () => <div>codex-oauth-quota</div>,
}));
vi.mock("@/lib/query/failover", () => ({
  useProviderHealth: () => ({ data: undefined }),
}));

function renderCard({
  official = false,
  templateType = "custom",
  enabled = true,
}: {
  official?: boolean;
  templateType?: TemplateType;
  enabled?: boolean;
} = {}) {
  const provider: Provider = {
    id: "cached-usage-provider",
    name: "Usage provider",
    category: official ? "official" : "custom",
    settingsConfig: {
      env: { ANTHROPIC_BASE_URL: "https://example.com" },
    },
    meta: {
      usage_script: { enabled, language: "javascript", code: "", templateType },
    },
  };
  const queryClient = createTestQueryClient();
  // A disabled React Query observer still receives existing cache entries.
  queryClient.setQueryData(usageKeys.script(provider.id, "claude"), {
    success: true,
    data: [
      { planName: "five_hour", total: 100, used: 0, remaining: 100, unit: "%" },
      { planName: "seven_day", total: 100, used: 30, remaining: 70, unit: "%" },
    ],
  });

  return render(
    <QueryClientProvider client={queryClient}>
      <ProviderCard
        provider={provider}
        appId="claude"
        isCurrent={true}
        presentation={{ chips: [], buttons: [] }}
        onEdit={vi.fn()}
        onDelete={vi.fn()}
        onConfigureUsage={vi.fn()}
        onOpenWebsite={vi.fn()}
        onDuplicate={vi.fn()}
      />
    </QueryClientProvider>,
  );
}

describe("ProviderCard cached usage expansion", () => {
  it("offers retry when a DSH ChatGPT provider is configured and its route is inactive", async () => {
    let active = false;
    server.use(
      http.post("http://tauri.local/auth_get_status", () =>
        HttpResponse.json({
          provider: "codex_oauth",
          authenticated: true,
          default_account_id: "fixture-account",
          accounts: [
            {
              id: "fixture-account",
              login: "Fixture account",
              reauth_required: false,
            },
          ],
        }),
      ),
      http.post("http://tauri.local/get_dsh_oauth_route_status", () =>
        HttpResponse.json({ active }),
      ),
    );
    const onActivate = vi.fn();
    const queryClient = createTestQueryClient();
    render(
      <QueryClientProvider client={queryClient}>
        <ProviderCard
          provider={{
            id: "chatgpt",
            name: "ChatGPT fixture",
            settingsConfig: { api: "openai-responses", models: {} },
            meta: {
              liveConfigManaged: true,
              providerType: "codex_oauth",
              authBinding: {
                source: "managed_account",
                authProvider: "codex_oauth",
                accountId: "fixture-account",
              },
            },
          }}
          appId="dsh"
          isCurrent={false}
          presentation={{ chips: [], buttons: [] }}
          dshOAuthRoute={{ activating: false, onActivate }}
          onEdit={vi.fn()}
          onDelete={vi.fn()}
          onConfigureUsage={vi.fn()}
          onOpenWebsite={vi.fn()}
        />
      </QueryClientProvider>,
    );
    await screen.findByText("dsh.codexOAuth.routeInactive");
    expect(screen.queryByText("dsh.codexOAuth.routeActive")).toBeNull();
    expect(screen.queryByText("codex-oauth-quota")).toBeNull();
    const retry = screen.getByRole("button", {
      name: "dsh.codexOAuth.retryActivation",
    });
    await waitFor(() => expect(retry).toBeEnabled());
    await userEvent.click(retry);
    expect(onActivate).toHaveBeenCalledOnce();
    active = true;
    await act(async () => {
      await queryClient.invalidateQueries({
        queryKey: ["dsh-oauth-route-status", "chatgpt"],
      });
    });
    await screen.findByText("dsh.codexOAuth.routeActive");
    expect(
      screen.queryByRole("button", { name: "dsh.codexOAuth.retryActivation" }),
    ).toBeNull();
  });

  it("keeps official subscription quota inline despite cached script tiers", () => {
    renderCard({ official: true, templateType: "official_subscription" });

    expect(screen.getByText("official-subscription-quota")).toBeInTheDocument();
    expect(screen.queryByText("expanded-plan-details")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "收起" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "展开" }),
    ).not.toBeInTheDocument();
  });

  it.each<Parameters<typeof renderCard>[0]>([
    { official: true },
    { templateType: "official_subscription" },
    { templateType: "token_plan" },
    { enabled: false },
  ])("does not expand ineligible cached usage: %j", (options) => {
    renderCard(options);

    expect(screen.queryByText("expanded-plan-details")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "收起" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "展开" }),
    ).not.toBeInTheDocument();
  });

  it("still expands ordinary multi-plan usage and allows collapsing it", async () => {
    const user = userEvent.setup();
    renderCard();

    expect(screen.getByText("expanded-plan-details")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "收起" }));
    expect(screen.queryByText("expanded-plan-details")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "展开" })).toBeInTheDocument();
  });
});
