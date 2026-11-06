import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ConfigDirectoryDetection } from "@/components/settings/ConfigDirectoryDetection";
import type { ConfigDirectoryDetection as DetectionResult } from "@/lib/api/settings";

const detectMock = vi.hoisted(() => vi.fn());

vi.mock("@/lib/api", () => ({
  settingsApi: { detectConfigDirectories: detectMock },
}));

vi.mock("react-i18next", async () => {
  const { default: messages } = await import("@/i18n/locales/en.json");
  return {
    useTranslation: () => ({
      t: (key: string, values?: { name?: string }) => {
        if (!key.startsWith("settings.")) return key;
        const text =
          messages.settings[
            key.slice("settings.".length) as keyof typeof messages.settings
          ];
        return typeof text === "string"
          ? text.replace("{{name}}", values?.name ?? "")
          : key;
      },
    }),
  };
});

const detection: DetectionResult = {
  currentDir: "C:/manual/codex",
  defaultDir: "C:/default/codex",
  candidates: [
    {
      path: "D:/detected/codex",
      sources: [
        "process:CODEX_HOME",
        "user:CODEX_HOME",
        "system:CODEX_HOME",
        "custom:CODEX_HOME",
      ],
      exists: true,
    },
    { path: "D:/missing/codex", sources: ["launcher"], exists: false },
    { path: "C:/manual/codex", sources: ["current"], exists: true },
    { path: "C:/default/codex", sources: ["default"], exists: true },
  ],
};

describe("ConfigDirectoryDetection", () => {
  beforeEach(() => detectMock.mockReset());

  it("shows candidate sources and adopts only after an explicit choice", async () => {
    detectMock.mockResolvedValue(detection);
    const onAdopt = vi.fn();
    render(<ConfigDirectoryDetection app="codex" onAdopt={onAdopt} />);

    await screen.findByText(/Process environment variable CODEX_HOME/);
    expect(
      screen.getByText(/User environment variable CODEX_HOME/),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/System environment variable CODEX_HOME/),
    ).toBeInTheDocument();
    expect(screen.getByText("Current setting")).toBeInTheDocument();
    expect(screen.getByText("Default location")).toBeInTheDocument();
    expect(screen.getByText(/custom:CODEX_HOME/)).toBeInTheDocument();
    expect(screen.getByText("launcher")).toBeInTheDocument();
    expect(screen.getAllByText("C:/manual/codex")[0]).toBeInTheDocument();
    expect(onAdopt).not.toHaveBeenCalled();
    expect(
      screen.getByRole("button", {
        name: "Use this directory D:/missing/codex",
      }),
    ).toBeDisabled();

    fireEvent.click(
      screen.getByRole("button", {
        name: "Use this directory D:/detected/codex",
      }),
    );
    expect(onAdopt).toHaveBeenCalledTimes(1);
    expect(onAdopt).toHaveBeenCalledWith("D:/detected/codex");
  });

  it("keeps applications without a directory override read-only", async () => {
    detectMock.mockResolvedValue({
      currentDir: "D:/native/dsh",
      defaultDir: "D:/native/dsh",
      candidates: [
        { path: "D:/native/dsh", sources: ["process:DSH_HOME"], exists: true },
      ],
    });
    render(<ConfigDirectoryDetection app="dsh" />);

    await screen.findByText("Process environment variable DSH_HOME");
    expect(screen.getAllByText("D:/native/dsh")[0]).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /Use this directory/ }),
    ).toBeNull();
  });

  it("can retry detection after a read failure", async () => {
    detectMock.mockRejectedValueOnce(new Error("unavailable"));
    detectMock.mockResolvedValueOnce(detection);
    render(<ConfigDirectoryDetection app="codex" />);

    await screen.findByRole("alert");
    fireEvent.click(screen.getByRole("button", { name: "Detect directories" }));
    await screen.findByText(/Process environment variable CODEX_HOME/);
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("ignores an older result after the saved directory changes", async () => {
    let finish!: (result: DetectionResult) => void;
    detectMock.mockReturnValueOnce(
      new Promise<DetectionResult>((resolve) => {
        finish = resolve;
      }),
    );
    detectMock.mockResolvedValueOnce({
      currentDir: "D:/saved/codex",
      defaultDir: "C:/default/codex",
      candidates: [],
    });
    const view = render(
      <ConfigDirectoryDetection app="codex" savedDirectory="C:/old/codex" />,
    );
    view.rerender(
      <ConfigDirectoryDetection app="codex" savedDirectory="D:/saved/codex" />,
    );
    await screen.findByText("D:/saved/codex");
    await act(async () => finish(detection));
    expect(screen.queryByText("C:/manual/codex")).toBeNull();
    expect(screen.getByText("D:/saved/codex")).toBeInTheDocument();
  });
});
