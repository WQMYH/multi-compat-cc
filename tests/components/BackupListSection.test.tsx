import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClientProvider } from "@tanstack/react-query";
import { http, HttpResponse } from "msw";
import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";
import { BackupListSection } from "@/components/settings/BackupListSection";
import { server } from "../msw/server";
import { createTestQueryClient } from "../utils/testQueryClient";

describe("BackupListSection", () => {
  const originalScrollIntoView = Element.prototype.scrollIntoView;

  beforeAll(() => {
    Element.prototype.scrollIntoView ??= vi.fn();
  });

  afterAll(() => {
    if (originalScrollIntoView) {
      Element.prototype.scrollIntoView = originalScrollIntoView;
    } else {
      Reflect.deleteProperty(Element.prototype, "scrollIntoView");
    }
  });

  it("shows fixed database retention and keeps separate tool retention and backup actions", async () => {
    const createBackup = vi.fn();
    server.use(
      http.post("http://tauri.local/list_db_backups", () =>
        HttpResponse.json([]),
      ),
      http.post("http://tauri.local/list_backup_locations", () =>
        HttpResponse.json([]),
      ),
      http.post("http://tauri.local/create_db_backup", () => {
        createBackup();
        return HttpResponse.json("fixture.db");
      }),
    );
    const user = userEvent.setup();
    const onSettingsChange = vi.fn();
    render(
      <QueryClientProvider client={createTestQueryClient()}>
        <BackupListSection
          backupIntervalHours={24}
          backupRetainCount={10}
          onSettingsChange={onSettingsChange}
        />
      </QueryClientProvider>,
    );

    expect(
      screen.getByText(
        "CC Switch keeps only the latest database backup it creates. Renamed or unrecognized files are kept during automatic cleanup.",
      ),
    ).toBeVisible();
    const interval = screen.getByRole("combobox", {
      name: "Automatic database backup interval",
    });
    const toolRetention = screen.getByRole("combobox", {
      name: "Hermes and OpenClaw configuration backups to keep",
    });
    expect(toolRetention).toHaveTextContent("10");
    expect(screen.getAllByRole("combobox")).toHaveLength(2);

    await user.click(toolRetention);
    await user.click(await screen.findByRole("option", { name: "5" }));
    expect(onSettingsChange).toHaveBeenNthCalledWith(1, {
      backupRetainCount: 5,
    });

    await user.click(interval);
    await user.click(await screen.findByRole("option", { name: "Disabled" }));
    expect(onSettingsChange).toHaveBeenNthCalledWith(2, {
      backupIntervalHours: 0,
    });

    await user.click(screen.getByRole("button", { name: "Backup Now" }));
    await waitFor(() => expect(createBackup).toHaveBeenCalledOnce());
  });
});
