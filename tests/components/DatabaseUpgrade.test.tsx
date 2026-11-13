import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { DatabaseUpgrade } from "@/components/DatabaseUpgrade";

const { invokeMock, listenMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listenMock: vi.fn(),
}));

vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));
vi.mock("@tauri-apps/api/event", () => ({ listen: listenMock }));
vi.mock("@tauri-apps/plugin-process", () => ({ exit: vi.fn() }));
vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

const releasesUrl = "https://github.com/WQMYH/multi-compat-cc/releases";

describe("DatabaseUpgrade update routing", () => {
  beforeEach(() => {
    invokeMock.mockReset();
    listenMock.mockReset();
    listenMock.mockResolvedValue(vi.fn());
  });

  it("opens the owned release page for an externally managed installation", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "is_portable_mode") return true;
      if (command === "check_app_update_available") return "4.0.32";
      return true;
    });
    render(<DatabaseUpgrade payload={{}} />);

    fireEvent.click(
      await screen.findByRole("button", { name: "dbUpgrade.openReleases" }),
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("open_external", {
        url: releasesUrl,
      }),
    );
    expect(invokeMock).not.toHaveBeenCalledWith("install_update_and_restart");
    expect(listenMock).not.toHaveBeenCalled();
  });

  it("allows the MSI path only after a fresh detection", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "is_portable_mode") return false;
      if (command === "check_app_update_available") return "4.0.32";
      if (command === "install_update_and_restart") return false;
      return true;
    });
    render(<DatabaseUpgrade payload={{}} />);

    fireEvent.click(
      await screen.findByRole("button", { name: "dbUpgrade.upgradeNow" }),
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("install_update_and_restart"),
    );
    expect(
      invokeMock.mock.calls.filter(
        ([command]) => command === "is_portable_mode",
      ),
    ).toHaveLength(2);
  });

  it("routes to the release page if MSI registration changes before the click", async () => {
    let detections = 0;
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "is_portable_mode") return ++detections > 1;
      if (command === "check_app_update_available") return "4.0.32";
      return true;
    });
    render(<DatabaseUpgrade payload={{}} />);

    fireEvent.click(
      await screen.findByRole("button", { name: "dbUpgrade.upgradeNow" }),
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("open_external", {
        url: releasesUrl,
      }),
    );
    expect(invokeMock).not.toHaveBeenCalledWith("install_update_and_restart");
  });

  it("fails closed when installation detection fails", async () => {
    invokeMock.mockImplementation(async (command: string) => {
      if (command === "is_portable_mode") throw new Error("unavailable");
      if (command === "check_app_update_available") return "4.0.32";
      return true;
    });
    render(<DatabaseUpgrade payload={{}} />);

    fireEvent.click(
      await screen.findByRole("button", { name: "dbUpgrade.openReleases" }),
    );
    await waitFor(() =>
      expect(invokeMock).toHaveBeenCalledWith("open_external", {
        url: releasesUrl,
      }),
    );
    expect(invokeMock).not.toHaveBeenCalledWith("install_update_and_restart");
  });
});
