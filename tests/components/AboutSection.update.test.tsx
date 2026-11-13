import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AboutSection } from "@/components/settings/AboutSection";

const { settingsMock } = vi.hoisted(() => ({
  settingsMock: {
    openExternal: vi.fn(),
    openPortableUpdateDownload: vi.fn(),
    installUpdateAndRestart: vi.fn(),
    checkUpdates: vi.fn(),
  },
}));

vi.mock("@/lib/api", () => ({ settingsApi: settingsMock }));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: async () => "4.0.31" }));
vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));
vi.mock("@/contexts/UpdateContext", () => ({
  useUpdate: () => ({
    hasUpdate: true,
    updateInfo: { availableVersion: "4.0.32", notes: "" },
    checkUpdate: vi.fn(),
    resetDismiss: vi.fn(),
    isChecking: false,
  }),
}));
vi.mock("@/components/WhatsNewDialog", () => ({ WhatsNewDialog: () => null }));

describe("AboutSection update routing", () => {
  beforeEach(() => {
    localStorage.clear();
    vi.clearAllMocks();
    settingsMock.openExternal.mockResolvedValue(true);
    settingsMock.openPortableUpdateDownload.mockResolvedValue(true);
  });

  it("opens the externally managed update path instead of installing", async () => {
    render(<AboutSection isPortable />);

    fireEvent.click(
      screen.getByRole("button", { name: "dbUpgrade.openReleases" }),
    );
    await waitFor(() =>
      expect(settingsMock.openPortableUpdateDownload).toHaveBeenCalledWith(
        "4.0.32",
      ),
    );
    expect(settingsMock.installUpdateAndRestart).not.toHaveBeenCalled();
    expect(screen.queryByText("settings.portableUpdateHint")).toBeNull();
  });

  it("uses the fork for release notes and retains the upstream project link", async () => {
    render(<AboutSection isPortable />);

    fireEvent.click(
      screen.getByRole("button", { name: "settings.releaseNotes" }),
    );
    await waitFor(() =>
      expect(settingsMock.openExternal).toHaveBeenCalledWith(
        "https://github.com/WQMYH/multi-compat-cc/releases/tag/v4.0.32",
      ),
    );
    fireEvent.click(
      screen.getByRole("button", { name: "settings.starOnGithub" }),
    );
    expect(settingsMock.openExternal).toHaveBeenCalledWith(
      "https://github.com/farion1231/cc-switch",
    );
  });
});
