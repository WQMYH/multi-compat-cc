import { describe, expect, it } from "vitest";
import {
  APP_IDS,
  orderAppIds,
  isAdditiveAppId,
  SKILLS_APP_IDS,
  MCP_APP_IDS,
  isProxyAppId,
  isModeAppId,
  isStackAppId,
} from "@/config/appConfig";
import { PROMPT_APP_IDS } from "@/lib/query/prompts";

describe("appConfig provider lifecycle", () => {
  it.each([
    "opencode",
    "openclaw",
    "hermes",
    "pi",
    "mcode",
    "kimi",
    "dsh",
    "zcode",
    "omp",
    "copilot",
    "workbuddy",
  ])("classifies %s as additive", (appId) => {
    expect(isAdditiveAppId(appId)).toBe(true);
  });

  it.each(["claude", "claude-desktop", "codex", "gemini", "grokbuild"])(
    "does not classify %s as additive",
    (appId) => {
      expect(isAdditiveAppId(appId)).toBe(false);
    },
  );
});

describe("DeepSeek Harness capabilities", () => {
  it("exposes shared configuration and excludes routing and aggregation", () => {
    expect(isModeAppId("dsh")).toBe(false);
    expect(isStackAppId("dsh")).toBe(false);
    expect(isAdditiveAppId("dsh")).toBe(true);
    expect(SKILLS_APP_IDS).toContain("dsh");
    expect(MCP_APP_IDS).toContain("dsh");
    expect(PROMPT_APP_IDS).toContain("dsh");
  });
});

// Antigravity shares management UI, but has independent configuration and no CCS proxy.
describe("Antigravity capabilities", () => {
  it("supports skills and MCP with direct provider switching", () => {
    expect(SKILLS_APP_IDS).toContain("agy");
    expect(MCP_APP_IDS).toContain("agy");
    expect(isAdditiveAppId("agy")).toBe(false);
    expect(isProxyAppId("agy")).toBe(false);
  });
});

// WorkBuddy manages providers, prompts, MCP (~/.workbuddy/mcp.json) and Skills
// (~/.workbuddy/skills) but has no local proxy.
describe("WorkBuddy capabilities", () => {
  it("is additive with MCP and Skills but without the local proxy", () => {
    expect(isAdditiveAppId("workbuddy")).toBe(true);
    expect(SKILLS_APP_IDS).toContain("workbuddy");
    expect((MCP_APP_IDS as string[]).includes("workbuddy")).toBe(true);
    expect(isProxyAppId("workbuddy")).toBe(false);
  });
});

describe("orderAppIds", () => {
  it("falls back to the default order when nothing is saved", () => {
    expect(orderAppIds(undefined)).toEqual(APP_IDS);
    expect(orderAppIds([])).toEqual(APP_IDS);
  });

  it("keeps the saved order and drops unknown or repeated ids", () => {
    const saved = [...APP_IDS].reverse();
    expect(orderAppIds(["future-app", ...saved, "claude"])).toEqual(saved);
  });

  it("puts an app missing from the saved order after its default predecessor", () => {
    const saved = APP_IDS.filter((app) => app !== "kimi");
    const predecessor = APP_IDS[APP_IDS.indexOf("kimi") - 1];
    // 移动 Kimi 的默认前驱后，新补入的 Kimi 仍跟在该前驱后面。
    const moved = [...saved.filter((app) => app !== predecessor), predecessor];
    expect(orderAppIds(moved)).toEqual([...moved, "kimi"]);
    expect(orderAppIds(APP_IDS.filter((app) => app !== "claude"))[0]).toBe(
      "claude",
    );
  });
});
