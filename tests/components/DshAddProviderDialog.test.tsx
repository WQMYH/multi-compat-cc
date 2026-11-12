import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClientProvider } from "@tanstack/react-query";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { http, HttpResponse } from "msw";
import { AddProviderDialog } from "@/components/providers/AddProviderDialog";
import { dshApi, type DshAccountStatus } from "@/lib/api/dsh";
import { server } from "../msw/server";
import { createTestQueryClient } from "../utils/testQueryClient";

vi.mock("@/lib/api/dsh", () => ({
  dshApi: {
    accountStatus: vi.fn(),
    accountSignOut: vi.fn(),
    loginStart: vi.fn(),
    loginPoll: vi.fn(),
    loginCancel: vi.fn(),
  },
}));
vi.mock("@/components/providers/forms/CopilotAuthSection", () => ({
  CopilotAuthSection: () => null,
}));
vi.mock("@/components/providers/forms/XaiOAuthSection", () => ({
  XaiOAuthSection: () => null,
}));
vi.mock("@/components/providers/forms/AgyOAuthSection", () => ({
  AgyOAuthSection: () => null,
}));
vi.mock("@/components/providers/KimiAccountLogin", () => ({
  KimiAccountLogin: () => null,
}));
vi.mock("@/components/JsonEditor", () => ({
  default: () => null,
}));

beforeAll(() => {
  Element.prototype.scrollIntoView ??= vi.fn();
  window.matchMedia ??= vi.fn().mockReturnValue({
    matches: false,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  });
});

const signedOut: DshAccountStatus = {
  signedIn: false,
  expired: false,
  issuer: null,
  profile: null,
  wallets: [],
  bonusWallets: [],
  usageUrl: null,
  topUpUrl: null,
  error: null,
};

describe("Account entries in the DeepSeek Harness add-provider dialog", () => {
  beforeEach(() => {
    vi.mocked(dshApi.accountStatus).mockResolvedValue(signedOut);
  });

  it("directs official sign-in to Auth Center without provider fields or an add footer", async () => {
    const onSubmit = vi.fn();
    render(
      <QueryClientProvider client={createTestQueryClient()}>
        <AddProviderDialog
          open
          appId="dsh"
          onOpenChange={vi.fn()}
          onSubmit={onSubmit}
        />
      </QueryClientProvider>,
    );

    fireEvent.click(
      await screen.findByRole("button", { name: "dsh.account.presetName" }),
    );
    const manage = await screen.findByRole("button", {
      name: "dsh.account.manageAccounts",
    });
    expect(
      screen.queryByRole("button", { name: "common.add" }),
    ).not.toBeInTheDocument();
    expect(screen.queryByLabelText("API Key")).not.toBeInTheDocument();

    fireEvent.click(manage);
    expect(
      await screen.findByRole("region", { name: "DeepSeek Harness" }),
    ).toBeInTheDocument();
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("opens the ChatGPT form from account login and saves the selected account and model", async () => {
    const accountId = "dsh-fixture-account";
    server.use(
      http.post("http://tauri.local/auth_get_status", () =>
        HttpResponse.json({
          provider: "codex_oauth",
          authenticated: true,
          default_account_id: accountId,
          accounts: [
            {
              id: accountId,
              login: "chatgpt-fixture@example.test",
              authenticated_at: 0,
              is_default: true,
              reauth_required: false,
            },
          ],
        }),
      ),
    );
    const onSubmit = vi.fn().mockResolvedValue(undefined);
    const user = userEvent.setup();
    render(
      <QueryClientProvider client={createTestQueryClient()}>
        <AddProviderDialog
          open
          appId="dsh"
          onOpenChange={vi.fn()}
          onSubmit={onSubmit}
        />
      </QueryClientProvider>,
    );

    await user.click(
      await screen.findByRole("button", {
        name: /providerPreset\.group\.login/,
      }),
    );
    expect(screen.getByText("providerPreset.loginWith.deepseek")).toBeVisible();
    expect(screen.getByText("providerPreset.loginWith.chatgpt")).toBeVisible();
    const search = screen.getByRole("textbox", {
      name: "providerPreset.searchAriaLabel",
    });
    for (const query of ["OpenAI", "ChatGPT"]) {
      await user.clear(search);
      await user.type(search, query);
      expect(
        screen.getByRole("button", { name: "OpenAI Official" }),
      ).toBeVisible();
    }
    await user.click(screen.getByRole("button", { name: "OpenAI Official" }));

    const add = await screen.findByRole("button", { name: "common.add" });
    expect(screen.queryByLabelText("API Key")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Add manually" }));
    const model = screen.getByPlaceholderText("Model ID");
    fireEvent.change(model, { target: { value: "fixture-model" } });
    fireEvent.blur(model);
    expect(add).toBeDisabled();

    const accountSelect = await screen.findByRole("combobox");
    expect(accountSelect).toHaveTextContent("选择一个 ChatGPT 账号");
    await user.click(accountSelect);
    expect(
      await screen.findAllByRole("option", { name: "选择一个 ChatGPT 账号" }),
    ).toHaveLength(1);
    await user.click(
      await screen.findByRole("option", {
        name: /chatgpt-fixture@example\.test/,
      }),
    );
    await waitFor(() => expect(add).toBeEnabled());
    await user.click(add);

    await waitFor(() =>
      expect(onSubmit).toHaveBeenCalledWith(
        expect.objectContaining({
          category: "custom",
          providerKey: "chatgpt",
          meta: expect.objectContaining({
            providerType: "codex_oauth",
            authBinding: {
              source: "managed_account",
              authProvider: "codex_oauth",
              accountId,
            },
          }),
          settingsConfig: {
            api: "openai-responses",
            name: "ChatGPT",
            options: {},
            models: { "fixture-model": { name: "" } },
          },
        }),
      ),
    );
  });
});

it("keeps the OpenAI API Key preset discoverable in the MCode provider picker", async () => {
  const user = userEvent.setup();
  render(
    <QueryClientProvider client={createTestQueryClient()}>
      <AddProviderDialog
        open
        appId="mcode"
        onOpenChange={vi.fn()}
        onSubmit={vi.fn()}
      />
    </QueryClientProvider>,
  );
  await user.type(
    await screen.findByRole("textbox", {
      name: "providerPreset.searchAriaLabel",
    }),
    "OpenAI",
  );
  expect(screen.getByRole("button", { name: "OpenAI API" })).toBeVisible();
});
