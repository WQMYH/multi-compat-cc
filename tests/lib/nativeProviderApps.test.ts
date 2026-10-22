import { describe, expect, it, vi } from "vitest";
import { providersApi } from "@/lib/api";
import {
  APP_IDS,
  DEFAULT_VISIBLE_APPS,
  MCP_APP_IDS,
  SKILLS_APP_IDS,
  isModeAppId,
  isStackAppId,
} from "@/config/appConfig";
import { providersQueryOptions } from "@/lib/query/queries";
import { TOOL_NAMES } from "@/components/apps/useToolManagement";

describe("native provider applications", () => {
  it.each(["dsh"] as const)(
    "uses the actual %s catalog rather than the Codex catalog",
    async (app) => {
      expect(APP_IDS).toContain(app);
      expect(DEFAULT_VISIBLE_APPS[app]).toBe(true);
      expect(providersQueryOptions(app).enabled).toBe(true);
      const getAll = vi.spyOn(providersApi, "getAll").mockResolvedValue({});
      const getCurrent = vi
        .spyOn(providersApi, "getCurrent")
        .mockResolvedValue("");
      await providersQueryOptions(app).queryFn();
      expect(getAll).toHaveBeenCalledOnce();
      expect(getAll).toHaveBeenCalledWith(app);
      expect(getCurrent).toHaveBeenCalledOnce();
      expect(getCurrent).toHaveBeenCalledWith(app);
      vi.restoreAllMocks();
      expect(MCP_APP_IDS).not.toContain(app);
      expect(SKILLS_APP_IDS).not.toContain(app);
    },
  );
  it("keeps the existing file-backed provider query enabled", () => {
    expect(providersQueryOptions("codex").enabled).toBe(true);
  });
  it("uses the shared mode controls for DSH", () => {
    expect(isModeAppId("dsh")).toBe(true);
    expect(isStackAppId("dsh")).toBe(true);
    expect(APP_IDS).not.toContain("qoder");
    expect(TOOL_NAMES).not.toContain("qodercn");
    expect(TOOL_NAMES).toContain("dsh");
    expect(isModeAppId("qoder")).toBe(false);
  });
});
