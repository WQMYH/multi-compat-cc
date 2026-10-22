import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { QoderGatewaySection } from "@/components/settings/QoderGatewaySection";
import {
  qoderGatewayApi,
  type QoderGatewayStatus,
} from "@/lib/api/qoderGateway";
import { proxyApi } from "@/lib/api/proxy";
import { chatgptPlanApi, type PlanAccount } from "@/lib/api/chatgptPlan";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (_key: string, options: { defaultValue: string }) =>
      options.defaultValue,
  }),
}));
vi.mock("@/lib/api/qoderGateway", () => ({
  qoderGatewayApi: {
    status: vi.fn(),
    save: vi.fn(),
    select: vi.fn(),
    remove: vi.fn(),
    prepareKey: vi.fn(),
    planModels: vi.fn(),
  },
}));
vi.mock("@/lib/api/proxy", () => ({
  proxyApi: { getProxyStatus: vi.fn(), startProxyServer: vi.fn() },
}));
vi.mock("@/lib/api/chatgptPlan", () => ({
  chatgptPlanApi: { status: vi.fn(), startLogin: vi.fn() },
}));

const empty: QoderGatewayStatus = {
  selectedId: null,
  keyConfigured: false,
  providers: [],
};
const configured: QoderGatewayStatus = {
  selectedId: "A",
  keyConfigured: true,
  providers: [
    {
      id: "A",
      name: "Fixture upstream",
      baseUrl: "https://provider.example/v1",
      apiFormat: "openai_chat",
      models: ["fixture-model"],
    },
  ],
};

describe("Qoder gateway configuration", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(qoderGatewayApi.status).mockResolvedValue(empty);
    vi.mocked(chatgptPlanApi.status).mockResolvedValue({
      accounts: [],
      activeId: null,
      pending: null,
    });
  });

  it("mount only reads configuration; saving a provider does not start a service or inference", async () => {
    const user = userEvent.setup();
    vi.mocked(qoderGatewayApi.save).mockResolvedValue(configured);
    render(<QoderGatewaySection />);
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Save gateway provider" }),
      ).toBeEnabled(),
    );
    expect(proxyApi.getProxyStatus).not.toHaveBeenCalled();
    expect(qoderGatewayApi.prepareKey).not.toHaveBeenCalled();
    await user.type(screen.getByLabelText("Provider name"), "Fixture upstream");
    await user.type(
      screen.getByLabelText("Upstream Base URL"),
      "https://provider.example/v1",
    );
    await user.type(
      screen.getByLabelText(/Upstream API key/),
      "synthetic-upstream-key",
    );
    fireEvent.change(screen.getByLabelText("Model IDs (one per line)"), {
      target: { value: "fixture-model\nfixture-second" },
    });
    await user.click(
      screen.getByRole("button", { name: "Save gateway provider" }),
    );
    await waitFor(() =>
      expect(qoderGatewayApi.save).toHaveBeenCalledWith({
        id: undefined,
        name: "Fixture upstream",
        baseUrl: "https://provider.example/v1",
        apiKey: "synthetic-upstream-key",
        apiFormat: "openai_chat",
        models: ["fixture-model", "fixture-second"],
      }),
    );
    expect(proxyApi.startProxyServer).not.toHaveBeenCalled();
    expect(qoderGatewayApi.prepareKey).not.toHaveBeenCalled();
  });

  it("starts the existing server and prepares only the local connection on explicit click", async () => {
    const user = userEvent.setup();
    vi.mocked(qoderGatewayApi.status).mockResolvedValue(configured);
    vi.mocked(proxyApi.getProxyStatus).mockResolvedValue({
      running: false,
      address: "127.0.0.1",
      port: 19399,
      active_connections: 0,
      total_requests: 0,
      success_requests: 0,
      failed_requests: 0,
      success_rate: 0,
      uptime_seconds: 0,
      current_provider: null,
      current_provider_id: null,
      last_request_at: null,
      last_error: null,
      failover_count: 0,
    });
    vi.mocked(proxyApi.startProxyServer).mockResolvedValue({
      address: "0.0.0.0",
      port: 19399,
      started_at: "2026-10-17T03:00:00Z",
    });
    vi.mocked(qoderGatewayApi.prepareKey).mockResolvedValue(
      "synthetic-local-key",
    );
    render(<QoderGatewaySection />);
    await waitFor(() =>
      expect(
        screen.getByRole("button", {
          name: "Start gateway and show connection",
        }),
      ).toBeEnabled(),
    );
    expect(proxyApi.startProxyServer).not.toHaveBeenCalled();
    await user.click(
      screen.getByRole("button", { name: "Start gateway and show connection" }),
    );
    expect(await screen.findByLabelText("Local gateway Base URL")).toHaveValue(
      "http://127.0.0.1:19399/qoder/v1",
    );
    expect(screen.getByLabelText("Local gateway key")).toHaveValue(
      "synthetic-local-key",
    );
    expect(proxyApi.startProxyServer).toHaveBeenCalledTimes(1);
    expect(qoderGatewayApi.prepareKey).toHaveBeenCalledTimes(1);
    expect(qoderGatewayApi.save).not.toHaveBeenCalled();
  });

  it("editing a provider does not retrieve the upstream key and blank preserves its existing reference", async () => {
    const user = userEvent.setup();
    vi.mocked(qoderGatewayApi.status).mockResolvedValue(configured);
    vi.mocked(qoderGatewayApi.save).mockResolvedValue(configured);
    render(<QoderGatewaySection />);
    await user.click(
      await screen.findByRole("button", { name: "Edit provider" }),
    );
    expect(screen.getByLabelText(/Upstream API key/)).toHaveValue("");
    await user.click(
      screen.getByRole("button", { name: "Save gateway provider" }),
    );
    await waitFor(() =>
      expect(qoderGatewayApi.save).toHaveBeenCalledWith({
        id: "A",
        name: "Fixture upstream",
        baseUrl: "https://provider.example/v1",
        apiKey: undefined,
        apiFormat: "openai_chat",
        models: ["fixture-model"],
      }),
    );
    expect(qoderGatewayApi.prepareKey).not.toHaveBeenCalled();
  });

  it("binds an explicit plan account rather than its active default and loads its real catalog only on click", async () => {
    const user = userEvent.setup();
    const account = (
      id: string,
      label: string,
      authorized: boolean,
    ): PlanAccount => ({
      id,
      label,
      phase: "connected",
      lastError: null,
      identity: {
        issuer: "https://auth.openai.com",
        clientId: "fixture-client",
        subject: id,
        email: null,
        inferenceAuthorized: authorized,
        expiresAt: 2000000000,
      },
    });
    const a = "a0956036-d2a2-4598-884f-cf5f1f1272ac";
    const b = "25a9da7c-e846-4637-b3cf-7478265a594b";
    vi.mocked(chatgptPlanApi.status).mockResolvedValue({
      accounts: [
        account(a, "Explicit plan", true),
        account(b, "Active default", true),
        account("identity-only", "Identity only", false),
      ],
      activeId: b,
      pending: null,
    });
    vi.mocked(qoderGatewayApi.planModels).mockResolvedValue([
      { id: "second-model", displayName: "Second display" },
      { id: "fixture-model", displayName: "Fixture display" },
    ]);
    vi.mocked(qoderGatewayApi.save).mockResolvedValue(empty);
    render(<QoderGatewaySection />);
    const selector = await screen.findByLabelText("Credential source");
    await waitFor(() =>
      expect(
        screen.getByRole("option", { name: /Explicit plan/ }),
      ).toBeEnabled(),
    );
    expect(
      screen.getByRole("option", { name: /Identity only/ }),
    ).toBeDisabled();
    expect(selector).toHaveValue("");
    expect(qoderGatewayApi.planModels).not.toHaveBeenCalled();
    expect(chatgptPlanApi.startLogin).not.toHaveBeenCalled();
    await user.selectOptions(selector, a);
    expect(screen.getByLabelText("Upstream Base URL")).toHaveValue(
      "https://api.openai.com/v1",
    );
    expect(screen.getByLabelText("Upstream Base URL")).toHaveAttribute(
      "readonly",
    );
    expect(screen.getByLabelText("Provider API")).toBeDisabled();
    expect(screen.queryByLabelText(/Upstream API key/)).not.toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: "Load this account's models" }),
    );
    expect(qoderGatewayApi.planModels).toHaveBeenCalledWith(a);
    expect(
      await screen.findByText("Second display · second-model"),
    ).toBeInTheDocument();
    await user.type(screen.getByLabelText("Provider name"), "Bound plan");
    await user.click(
      screen.getByRole("button", { name: "Save gateway provider" }),
    );
    await waitFor(() =>
      expect(qoderGatewayApi.save).toHaveBeenCalledWith({
        id: undefined,
        name: "Bound plan",
        baseUrl: "https://api.openai.com/v1",
        apiFormat: "openai_responses",
        apiKey: undefined,
        planAccountId: a,
        models: ["second-model", "fixture-model"],
      }),
    );
    expect(proxyApi.startProxyServer).not.toHaveBeenCalled();
  });

  it("can discover a newly signed-in sibling account without reopening auth or requesting models", async () => {
    const user = userEvent.setup();
    render(<QoderGatewaySection />);
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Refresh plan accounts" }),
      ).toBeEnabled(),
    );
    vi.mocked(chatgptPlanApi.status).mockResolvedValue({
      accounts: [
        {
          id: "new-account",
          label: "Newly signed in",
          phase: "reauth_required",
          identity: null,
          lastError: null,
        },
      ],
      activeId: null,
      pending: null,
    });
    await user.click(
      screen.getByRole("button", { name: "Refresh plan accounts" }),
    );
    expect(
      await screen.findByRole("option", { name: /Newly signed in/ }),
    ).toBeDisabled();
    expect(qoderGatewayApi.planModels).not.toHaveBeenCalled();
    expect(chatgptPlanApi.startLogin).not.toHaveBeenCalled();
  });
});
