import { piProviderPresets, type PiPresetModel } from "./piProviderPresets";
import type { McodeProviderPreset } from "./mcodeProviderPresets";
import { OPENAI_OFFICIAL_PRESET_DISPLAY } from "./codexProviderPresets";

// Configurable fields offered by dsh-llm-pi-ai@0.2.1-rc.2 COMPAT_GATES.
// Pi also has vendor-only switches; DSH rejects those in profile overrides.
const DSH_COMPAT_FIELDS: Record<string, ReadonlySet<string>> = {
  "openai-completions": new Set([
    "supportsStore",
    "supportsDeveloperRole",
    "supportsReasoningEffort",
    "supportsUsageInStreaming",
    "supportsFinishReason",
    "maxTokensField",
    "requiresToolResultName",
    "requiresAssistantAfterToolResult",
    "requiresThinkingAsText",
    "requiresReasoningContentOnAssistantMessages",
    "thinkingFormat",
    "chatTemplateKwargs",
    "chatTemplateArgs",
    "supportsThinkingTokenBudget",
    "thinkingTokenBudgetField",
    "vllmPriority",
    "supportsStrictMode",
    "cacheControlFormat",
    "supportsLongCacheRetention",
  ]),
  "openai-responses": new Set([
    "supportsDeveloperRole",
    "supportsMaxOutputTokens",
    "supportsStrictMode",
    "supportsLongCacheRetention",
  ]),
  "anthropic-messages": new Set([
    "supportsEagerToolInputStreaming",
    "supportsLongCacheRetention",
    "supportsCacheControlOnTools",
    "supportsTemperature",
    "forceAdaptiveThinking",
    "allowEmptySignature",
    "supportsStrictTools",
  ]),
};

function dshCompat(api: string, compat?: Record<string, unknown>) {
  const supported = Object.fromEntries(
    Object.entries(compat ?? {}).filter(([key]) =>
      DSH_COMPAT_FIELDS[api]?.has(key),
    ),
  );
  return Object.keys(supported).length > 0 ? { compat: supported } : {};
}

function reasoningEfforts(model: PiPresetModel) {
  if (!model.reasoning) return false;
  // An absent Pi base level is supported with its default wire value. Null
  // explicitly disables it; DSH instead declares only the offered levels.
  const defaults = {
    off: null,
    minimal: "minimal",
    low: "low",
    medium: "medium",
    high: "high",
  };
  return Object.fromEntries(
    Object.entries({ ...defaults, ...model.thinkingLevelMap }).filter(
      ([level, value]) =>
        value !== null ||
        (level === "off" && model.thinkingLevelMap?.off === undefined),
    ),
  );
}

/** DSH's pi-ai adapter accepts these three wire protocols and native compatibility options. */
export const dshProviderPresets: McodeProviderPreset[] =
  piProviderPresets.flatMap((preset) => {
    const config = preset.settingsConfig;
    if (
      ![
        "openai-completions",
        "openai-responses",
        "anthropic-messages",
      ].includes(config.api)
    ) {
      return [];
    }
    return [
      {
        ...preset,
        settingsConfig: {
          name: preset.name,
          kind: "custom",
          enabled: true,
          api: config.api,
          ...dshCompat(config.api, config.compat),
          options: {
            baseURL: config.baseUrl,
            apiKey: config.apiKey,
            ...(config.headers ? { headers: config.headers } : {}),
          },
          models: Object.fromEntries(
            config.models.map((model) => [
              model.id,
              {
                name: model.name,
                modalities: { input: model.input, output: ["text"] },
                limit: {
                  context: model.contextWindow,
                  output: model.maxTokens,
                },
                ...dshCompat(config.api, model.compat),
                ...(model.reasoning ? { reasoning: true } : {}),
                reasoningEfforts: reasoningEfforts(model),
              },
            ]),
          ),
        },
      },
    ];
  });

export const DSH_ACCOUNT_PRESET_ID = "dsh-official-account";
export const DSH_CODEX_OAUTH_PRESET_ID = "dsh-codex-oauth";

export const dshCodexOAuthPreset: McodeProviderPreset & {
  displayName: string;
  providerType: "codex_oauth";
} = {
  name: "ChatGPT",
  nameKey: "dsh.codexOAuth.presetName",
  displayName: OPENAI_OFFICIAL_PRESET_DISPLAY.name,
  providerKey: "chatgpt",
  websiteUrl: "https://chatgpt.com",
  icon: OPENAI_OFFICIAL_PRESET_DISPLAY.icon,
  iconColor: OPENAI_OFFICIAL_PRESET_DISPLAY.iconColor,
  category: "custom",
  providerType: "codex_oauth",
  settingsConfig: {
    name: "ChatGPT",
    kind: "custom",
    enabled: true,
    api: "openai-responses",
    options: { baseURL: "", apiKey: "" },
    models: {},
  },
};

// An action in the account-login group, separate from the API-key catalog:
// DSH serves the signed-in account's models itself, nothing is written here.
export const dshAccountPreset: McodeProviderPreset = {
  name: "DeepSeek",
  nameKey: "dsh.account.presetName",
  providerKey: DSH_ACCOUNT_PRESET_ID,
  websiteUrl: "https://platform.deepseek.com",
  icon: "deepseek",
  category: "official",
  settingsConfig: {
    name: "DeepSeek",
    kind: "custom",
    enabled: true,
    api: "openai-completions",
    options: { baseURL: "", apiKey: "" },
    models: {},
  },
};
