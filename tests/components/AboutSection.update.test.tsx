import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AboutSection } from "@/components/settings/AboutSection";

const { settingsMock, providersMock, updateMock } = vi.hoisted(() => ({
  settingsMock: {
    openExternal: vi.fn(),
    openPortableUpdateDownload: vi.fn(),
    installUpdateAndRestart: vi.fn(),
    checkUpdates: vi.fn(),
  },
  providersMock: { updateTrayMenu: vi.fn() },
  updateMock: {
    hasUpdate: true,
    updateInfo: { availableVersion: "4.0.32", notes: "" },
    checkUpdate: vi.fn(),
    resetDismiss: vi.fn(),
    isChecking: false,
    autoUpdateSupported: false,
    autoUpdateDownloading: false,
    readyUpdate: null,
    refreshAutoUpdate: vi.fn(),
    startAutoDownload: vi.fn(),
  },
}));

vi.mock("@/lib/api", () => ({
  settingsApi: settingsMock,
  providersApi: providersMock,
}));
vi.mock("@tauri-apps/api/app", () => ({ getVersion: async () => "4.0.31" }));
vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));
vi.mock("@/contexts/UpdateContext", () => ({
  useUpdate: () => updateMock,
}));
vi.mock("@/components/WhatsNewDialog", () => ({ WhatsNewDialog: () => null }));

describe("AboutSection update routing", () => {
  beforeEach(() => {
    localStorage.clear();
    vi.clearAllMocks();
    updateMock.autoUpdateSupported = false;
    settingsMock.openExternal.mockResolvedValue(true);
    settingsMock.openPortableUpdateDownload.mockResolvedValue(true);
    settingsMock.installUpdateAndRestart.mockResolvedValue(true);
  });

  it("opens the externally managed update path instead of installing", async () => {
    updateMock.autoUpdateSupported = true;
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

  it("opens releases for a managed EXE without MSI registration", async () => {
    render(<AboutSection isPortable={false} />);

    fireEvent.click(
      screen.getByRole("button", { name: "dbUpgrade.openReleases" }),
    );
    await waitFor(() =>
      expect(settingsMock.openPortableUpdateDownload).toHaveBeenCalledWith(
        "4.0.32",
      ),
    );
    expect(settingsMock.installUpdateAndRestart).not.toHaveBeenCalled();
  });

  it("uses the updater for an eligible MSI installation", async () => {
    updateMock.autoUpdateSupported = true;
    render(<AboutSection isPortable={false} />);

    fireEvent.click(screen.getByRole("button", { name: "settings.updateTo" }));
    await waitFor(() =>
      expect(settingsMock.installUpdateAndRestart).toHaveBeenCalledOnce(),
    );
    expect(settingsMock.openPortableUpdateDownload).not.toHaveBeenCalled();
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
