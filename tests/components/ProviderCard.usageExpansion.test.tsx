import { QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { ComponentProps } from "react";
import { http, HttpResponse } from "msw";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { ProviderCard } from "@/components/providers/ProviderCard";
import type { TemplateType } from "@/config/constants";
import { usageKeys } from "@/lib/query/usage";
import type { Provider } from "@/types";
import { createTestQueryClient } from "../utils/testQueryClient";
import { server } from "../msw/server";

vi.mock("@/components/providers/ProviderActions", () => ({
  ProviderActions: () => null,
}));
vi.mock("@/components/ProviderIcon", () => ({
  ProviderIcon: ({ icon }: { icon?: string }) => (
    <span data-testid="provider-icon">{icon ?? "fallback"}</span>
  ),
}));
vi.mock("@/components/UsageFooter", async (importOriginal) => {
  const { default: UsageFooter } =
    await importOriginal<typeof import("@/components/UsageFooter")>();
  return {
    default: (props: ComponentProps<typeof UsageFooter>) =>
      props.provider.meta?.providerType === "dsh-native-balance" ? (
        <UsageFooter {...props} />
      ) : props.inline ? null : (
        <div>expanded-plan-details</div>
      ),
  };
});
vi.mock("@/components/SubscriptionQuotaFooter", () => ({
  default: () => <div>official-subscription-quota</div>,
}));
vi.mock("@/lib/query/failover", () => ({
  useProviderHealth: () => ({ data: undefined }),
}));

function renderCard({
  official = false,
  templateType = "custom",
  enabled = true,
  appId = "claude",
  nativeBalance = false,
  nativeProfile = "desktop",
  nativeProvider,
}: {
  official?: boolean;
  templateType?: TemplateType;
  enabled?: boolean;
  appId?: "claude" | "dsh";
  nativeBalance?: boolean;
  nativeProfile?: "desktop" | "web";
  nativeProvider?: string;
} = {}) {
  const provider: Provider = {
    id: nativeBalance
      ? `native-current:${nativeProfile}`
      : "cached-usage-provider",
    name: "Usage provider",
    category: official ? "official" : "custom",
    settingsConfig: {
      env: { ANTHROPIC_BASE_URL: "https://example.com" },
      ...(nativeBalance
        ? {
            profile: nativeProfile,
            nativeRef: true,
            nativeBalanceId: `native-current:${nativeProfile}:fixture-revision`,
          }
        : {}),
      ...(nativeProvider
        ? { provider: nativeProvider, model: "deepseek-flash" }
        : {}),
    },
    meta: {
      ...(nativeBalance ? { providerType: "dsh-native-balance" } : {}),
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

  const view = render(
    <QueryClientProvider client={queryClient}>
      <ProviderCard
        provider={provider}
        appId={appId}
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
  return { ...view, queryClient };
}

describe("ProviderCard cached usage expansion", () => {
  it("uses the existing DeepSeek icon and a readable native name without changing the provider identity", () => {
    i18n.addResourceBundle(
      "zh",
      "translation",
      { provider: zh.provider },
      true,
      true,
    );
    renderCard({
      appId: "dsh",
      nativeBalance: true,
      nativeProvider: "deepseek-official",
      enabled: false,
    });
    expect(
      screen.getByRole("heading", { name: "DeepSeek 官方 API · Flash" }),
    ).toBeVisible();
    expect(screen.getByTestId("provider-icon")).toHaveTextContent("deepseek");
  });

  it.each([
    ["desktop", "桌面端 · Desktop"],
    ["web", "网页端 · Web"],
  ] as const)(
    "labels native DSH %s separately from its truncated title",
    (nativeProfile, label) => {
      i18n.addResourceBundle(
        "zh",
        "translation",
        { provider: zh.provider },
        true,
        true,
      );
      renderCard({
        appId: "dsh",
        nativeBalance: true,
        nativeProfile,
        enabled: false,
      });
      expect(screen.getByText(label)).toBeVisible();
      expect(
        screen.getByRole("heading", { name: "Usage provider" }),
      ).toBeVisible();
    },
  );

  it("renders registered native DSH balances through the existing usage footer", async () => {
    i18n.addResourceBundle(
      "zh",
      "translation",
      { quota: zh.quota },
      true,
      true,
    );
    const calls: unknown[] = [];
    server.use(
      http.post(
        "http://tauri.local/queryProviderUsage",
        async ({ request }) => {
          calls.push(await request.json());
          return HttpResponse.json({
            success: true,
            data: [
              { planName: "CNY", remaining: 31.25, unit: "CNY", isValid: true },
            ],
          });
        },
      ),
    );
    const { queryClient } = renderCard({
      official: true,
      appId: "dsh",
      nativeBalance: true,
      templateType: "balance",
    });
    expect(await screen.findByText(/31\.25/)).toBeInTheDocument();
    expect(calls).toEqual([
      { app: "dsh", providerId: "native-current:desktop:fixture-revision" },
    ]);
    expect(
      screen.queryByText("official-subscription-quota"),
    ).not.toBeInTheDocument();
    const invalidation = vi.spyOn(queryClient, "invalidateQueries");
    server.use(
      http.post("http://tauri.local/queryProviderUsage", () =>
        HttpResponse.json({
          success: false,
          error: "DSH_NATIVE_BALANCE_IDENTITY_CHANGED",
        }),
      ),
    );
    await userEvent
      .setup()
      .click(screen.getByRole("button", { name: /31\.25/ }));
    expect(
      await screen.findByText("DSH_NATIVE_BALANCE_IDENTITY_CHANGED"),
    ).toBeInTheDocument();
    expect(screen.queryByText(/31\.25/)).not.toBeInTheDocument();
    expect(invalidation).toHaveBeenCalledWith({
      queryKey: ["providers", "dsh", "nativeDefaults"],
    });
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
