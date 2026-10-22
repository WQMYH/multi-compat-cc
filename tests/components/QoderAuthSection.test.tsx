import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { QoderAuthSection } from "@/components/settings/QoderAuthSection";
import { qoderAuthApi, type QoderAuthProfile } from "@/lib/api/qoderAuth";

vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));

vi.mock("@/lib/api/qoderAuth", () => ({
  qoderAuthApi: {
    list: vi.fn(),
    save: vi.fn(),
    verify: vi.fn(),
    select: vi.fn(),
    remove: vi.fn(),
    openCli: vi.fn(),
  },
}));

const profile: QoderAuthProfile = {
  id: "A",
  label: "Qoder fixture",
  source: { type: "pat_env", envVar: "FIXTURE_PAT" },
  cliPath: "/fixture/qoderclicn",
  revision: 1,
  account: null,
  verifiedAt: null,
  lastError: null,
};

describe("Qoder authorization profile surface", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("does not infer login on mount or allow selecting an unverified identity", async () => {
    vi.mocked(qoderAuthApi.list).mockResolvedValue({
      profiles: [profile],
      selectedId: null,
    });
    const verified = {
      ...profile,
      revision: 2,
      account: { userId: "account-A" },
      verifiedAt: "2026-10-17T00:00:00Z",
    };
    vi.mocked(qoderAuthApi.verify).mockResolvedValue({
      profiles: [verified],
      selectedId: null,
    });
    vi.mocked(qoderAuthApi.select).mockResolvedValue({
      profiles: [verified],
      selectedId: "A",
    });
    const user = userEvent.setup();
    render(<QoderAuthSection />);
    expect(await screen.findByText("Account not verified")).toBeInTheDocument();
    expect(qoderAuthApi.verify).not.toHaveBeenCalled();
    expect(
      screen.getByRole("button", { name: "Select profile" }),
    ).toBeDisabled();
    await user.click(
      screen.getByRole("button", { name: "Verify account (no inference)" }),
    );
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Select profile" }),
      ).toBeEnabled(),
    );
    await user.click(screen.getByRole("button", { name: "Select profile" }));
    expect(qoderAuthApi.verify).toHaveBeenCalledTimes(1);
    expect(qoderAuthApi.verify).toHaveBeenCalledWith("A");
    expect(qoderAuthApi.select).toHaveBeenCalledTimes(1);
    expect(qoderAuthApi.select).toHaveBeenCalledWith("A");
  });

  it("saves only an explicit source reference and does not start an account probe", async () => {
    vi.mocked(qoderAuthApi.list).mockResolvedValue({
      profiles: [],
      selectedId: null,
    });
    vi.mocked(qoderAuthApi.save).mockResolvedValue({
      profiles: [profile],
      selectedId: null,
    });
    const user = userEvent.setup();
    render(<QoderAuthSection />);
    await waitFor(() =>
      expect(screen.getByLabelText("Profile name")).toBeEnabled(),
    );
    await user.type(screen.getByLabelText("Profile name"), "Qoder fixture");
    await user.type(
      screen.getByLabelText(/Qoder CN CLI executable/),
      "/fixture/qoderclicn",
    );
    await user.type(
      screen.getByLabelText("Environment variable name (not the token)"),
      "FIXTURE_PAT",
    );
    await user.click(
      screen.getByRole("button", { name: "Save profile reference" }),
    );
    expect(qoderAuthApi.save).toHaveBeenCalledWith({
      id: undefined,
      label: "Qoder fixture",
      cliPath: "/fixture/qoderclicn",
      nodePath: undefined,
      source: { type: "pat_env", envVar: "FIXTURE_PAT" },
    });
    expect(qoderAuthApi.verify).not.toHaveBeenCalled();
  });

  it("opens only the chosen native account terminal on explicit click without claiming login", async () => {
    const native = {
      ...profile,
      id: "native-B",
      label: "Native fixture",
      source: { type: "cli" as const, configDir: "/fixture/native-B" },
    };
    vi.mocked(qoderAuthApi.list).mockResolvedValue({
      profiles: [profile, native],
      selectedId: null,
    });
    vi.mocked(qoderAuthApi.openCli).mockResolvedValue(undefined);
    const user = userEvent.setup();
    render(<QoderAuthSection />);
    const open = await screen.findByRole("button", {
      name: "Open official account CLI",
    });
    expect(qoderAuthApi.openCli).not.toHaveBeenCalled();
    expect(
      screen.getAllByRole("button", { name: "Open official account CLI" }),
    ).toHaveLength(1);
    await user.click(open);
    expect(await screen.findByRole("status")).toHaveTextContent(
      "Complete /login, /status or /logout there, then verify this profile",
    );
    expect(qoderAuthApi.openCli).toHaveBeenCalledTimes(1);
    expect(qoderAuthApi.openCli).toHaveBeenCalledWith("native-B");
    expect(qoderAuthApi.verify).not.toHaveBeenCalled();
    expect(qoderAuthApi.select).not.toHaveBeenCalled();
    expect(qoderAuthApi.save).not.toHaveBeenCalled();
    expect(qoderAuthApi.remove).not.toHaveBeenCalled();
    expect(screen.getAllByText("Account not verified")).toHaveLength(2);
  });
});
