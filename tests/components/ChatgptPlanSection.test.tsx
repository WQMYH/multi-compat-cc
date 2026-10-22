import {
  act,
  cleanup,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ChatgptPlanSection } from "@/components/settings/ChatgptPlanSection";
import { chatgptPlanApi, type PlanAccounts } from "@/lib/api/chatgptPlan";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));
vi.mock("@/lib/api/chatgptPlan", () => ({
  chatgptPlanApi: {
    status: vi.fn(),
    startLogin: vi.fn(),
    cancelLogin: vi.fn(),
    select: vi.fn(),
    signOut: vi.fn(),
  },
}));

const empty: PlanAccounts = { accounts: [], activeId: null, pending: null };
const account = {
  id: "registration-a",
  label: "Personal",
  phase: "connected" as const,
  identity: {
    issuer: "https://auth.openai.com",
    clientId: "synthetic-client",
    subject: "synthetic-user",
    email: "synthetic@example.test",
    inferenceAuthorized: false,
    expiresAt: 2000000000,
  },
  lastError: null,
};

describe("explicit ChatGPT plan account controls", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });
  afterEach(() => {
    cleanup();
    vi.useRealTimers();
  });

  it("only reads on mount and opens an explicit sign-in without receiving tokens or running inference", async () => {
    vi.mocked(chatgptPlanApi.status).mockResolvedValue(empty);
    vi.mocked(chatgptPlanApi.startLogin).mockResolvedValue({
      attemptId: "attempt-a",
      accountId: "registration-a",
    });
    const user = userEvent.setup();
    render(<ChatgptPlanSection />);
    await waitFor(() =>
      expect(screen.getByLabelText("Registration name")).toBeEnabled(),
    );
    expect(chatgptPlanApi.startLogin).not.toHaveBeenCalled();
    expect(chatgptPlanApi.select).not.toHaveBeenCalled();
    await user.type(screen.getByLabelText("Registration name"), "Personal");
    await user.click(
      screen.getByRole("button", { name: "Continue with ChatGPT" }),
    );
    await waitFor(() =>
      expect(chatgptPlanApi.startLogin).toHaveBeenCalledWith(
        "Personal",
        undefined,
      ),
    );
    expect(chatgptPlanApi.startLogin).toHaveBeenCalledTimes(1);
  });

  it("distinguishes identity-only consent and blocks selecting an unknown refresh while preserving its explicit recovery", async () => {
    vi.mocked(chatgptPlanApi.status).mockResolvedValue({
      ...empty,
      accounts: [
        account,
        {
          ...account,
          id: "registration-b",
          label: "Work",
          phase: "refresh_unknown",
          lastError: "refresh_interrupted_unknown",
        },
      ],
    });
    render(<ChatgptPlanSection />);
    expect(
      await screen.findByText("Signed in; plan use not authorized"),
    ).toBeInTheDocument();
    const rows = screen.getAllByRole("listitem");
    expect(
      within(rows[0]).getByRole("button", { name: "Select account" }),
    ).toBeEnabled();
    expect(
      within(rows[1]).getByRole("button", { name: "Select account" }),
    ).toBeDisabled();
    expect(
      within(rows[1]).getByRole("button", { name: "Authorize again" }),
    ).toBeEnabled();
    expect(chatgptPlanApi.startLogin).not.toHaveBeenCalled();
    expect(chatgptPlanApi.signOut).not.toHaveBeenCalled();
  });

  it("observes callback completion automatically without reopening sign-in or changing another account", async () => {
    vi.useFakeTimers();
    vi.mocked(chatgptPlanApi.status).mockResolvedValue({
      ...empty,
      pending: { attemptId: "attempt-a", accountId: "registration-a" },
    });
    render(<ChatgptPlanSection />);
    await act(async () => {
      await Promise.resolve();
    });
    expect(
      screen.getByRole("button", { name: "Cancel sign-in" }),
    ).toBeEnabled();
    vi.mocked(chatgptPlanApi.status).mockResolvedValue({
      ...empty,
      activeId: account.id,
      accounts: [account],
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1000);
    });
    expect(
      screen.queryByRole("button", { name: "Cancel sign-in" }),
    ).not.toBeInTheDocument();
    expect(screen.getByText(/Personal.*Selected/)).toBeInTheDocument();
    expect(chatgptPlanApi.startLogin).not.toHaveBeenCalled();
    expect(chatgptPlanApi.select).not.toHaveBeenCalled();
  });
});
