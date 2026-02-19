import type {
  SimplifiedModel,
  ModelType,
  CodeChatModel,
} from "../../../../../services/refact";
import type { CapsResponse, CapCost } from "../../../../../services/refact";

export type UiModel = SimplifiedModel & {
  modelType: ModelType;
  pricing?: CapCost;
  pricingLabel?: string;
  nCtx?: number;
  nCtxLabel?: string;
  isDefault?: boolean;
  isLight?: boolean;
  isThinking?: boolean;
  capabilities?: ModelCapabilities;
};

export type ModelCapabilities = {
  supportsTools?: boolean;
  supportsMultimodality?: boolean;
  supportsClicks?: boolean;
  supportsAgent?: boolean;
  reasoningEffortOptions?: string[] | null;
  supportsThinkingBudget?: boolean;
  supportsAdaptiveThinkingBudget?: boolean;
};

export type ModelGroup = {
  id: string;
  title: string;
  description?: string;
  models: UiModel[];
};

/**
 * Format context window size to human-readable format
 */
export function formatContextWindow(nCtx: number): string {
  if (nCtx >= 1_000_000) {
    return `${(nCtx / 1_000_000).toFixed(1)}M`.replace(".0M", "M");
  }
  if (nCtx >= 1_000) {
    return `${Math.round(nCtx / 1_000)}K`;
  }
  return nCtx.toString();
}

export function formatPricing(cost: CapCost, compact = true): string {
  const fmt = (n?: number) => {
    if (typeof n !== "number" || !Number.isFinite(n)) return "–";
    if (n >= 1) return `$${n.toFixed(2)}`;
    if (n >= 0.01) return `$${n.toFixed(2)}`;
    return `$${n.toFixed(3)}`;
  };

  if (compact) {
    return `${fmt(cost.prompt)}/${fmt(cost.generated)}`;
  }

  const parts = [
    `input: ${fmt(cost.prompt)}`,
    `output: ${fmt(cost.generated)}`,
  ];

  if (typeof cost.cache_read === "number" && Number.isFinite(cost.cache_read)) {
    parts.push(`cache read: ${fmt(cost.cache_read)}`);
  }
  if (
    typeof cost.cache_creation === "number" &&
    Number.isFinite(cost.cache_creation)
  ) {
    parts.push(`cache create: ${fmt(cost.cache_creation)}`);
  }

  return parts.join(" • ") + " per 1M tokens";
}

/**
 * Try to find the pricing key in caps.metadata.pricing that corresponds to a given model.
 * Backend inserts pricing under both fully-qualified keys (provider/model) and bare model names.
 */
function pickPricingKey(args: {
  caps: CapsResponse;
  modelName: string;
  providerName?: string;
}): string | null {
  const { caps, modelName, providerName } = args;
  const pricing = caps.metadata?.pricing;
  if (!pricing) return null;

  const hasKey = (key: string) =>
    Object.prototype.hasOwnProperty.call(pricing, key);

  // 1. Try exact match first (handles both bare and qualified names)
  if (hasKey(modelName)) {
    return modelName;
  }

  // 2. Try fully-qualified key if we have provider context
  if (providerName) {
    const qualifiedKey = `${providerName}/${modelName}`;
    if (hasKey(qualifiedKey)) {
      return qualifiedKey;
    }
  }

  // 3. Try stripping any provider prefix (e.g., "openai/gpt-4o" -> "gpt-4o")
  if (modelName.includes("/")) {
    const bareModel = modelName.split("/").pop();
    if (bareModel && hasKey(bareModel)) {
      return bareModel;
    }
  }

  // 4. For multi-slash names (e.g., "openrouter/anthropic/claude-3-5-sonnet"),
  //    try the last two segments as a key
  const segments = modelName.split("/");
  if (segments.length > 2) {
    const lastTwoSegments = segments.slice(-2).join("/");
    if (hasKey(lastTwoSegments)) {
      return lastTwoSegments;
    }
  }

  return null;
}

/**
 * Extract capabilities from chat model
 */
function extractCapabilities(
  capsModel: CodeChatModel | undefined,
): ModelCapabilities | undefined {
  if (!capsModel || typeof capsModel !== "object") return undefined;

  return {
    supportsTools: capsModel.supports_tools,
    supportsMultimodality: capsModel.supports_multimodality,
    supportsClicks: capsModel.supports_clicks,
    supportsAgent: capsModel.supports_agent,
    reasoningEffortOptions: capsModel.reasoning_effort_options,
    supportsThinkingBudget: capsModel.supports_thinking_budget,
    supportsAdaptiveThinkingBudget: capsModel.supports_adaptive_thinking_budget,
  };
}

/**
 * Attach pricing, context window & capability flags to each simplified model.
 * Works even if caps/metadata/pricing is missing.
 */
export function attachPricingAndCapabilities(
  models: SimplifiedModel[],
  { caps, modelType }: { caps?: CapsResponse; modelType: ModelType },
): UiModel[] {
  if (!caps) {
    // No caps → only attach modelType
    return models.map((m) => ({ ...m, modelType }));
  }

  const capsModels =
    modelType === "chat" ? caps.chat_models : caps.completion_models;

  return models.map((m) => {
    const capsModelKey = `refact/${m.name}`;
    const capsModel = capsModels[capsModelKey];

    const pricingKey = pickPricingKey({
      caps,
      modelName: m.name,
    });

    const pricing =
      pricingKey && caps.metadata?.pricing
        ? caps.metadata.pricing[pricingKey]
        : undefined;

    // eslint-disable-next-line @typescript-eslint/no-unnecessary-condition
    const nCtx = capsModel?.n_ctx;

    const uiModel: UiModel = {
      ...m,
      modelType,
      pricing,
      pricingLabel: pricing ? formatPricing(pricing) : undefined,
      nCtx,
      nCtxLabel: nCtx ? formatContextWindow(nCtx) : undefined,
    };

    // Chat-type specific flags
    if (modelType === "chat") {
      uiModel.isDefault = caps.chat_default_model === `refact/${m.name}`;
      uiModel.isLight = caps.chat_light_model === `refact/${m.name}`;
      uiModel.isThinking = caps.chat_thinking_model === `refact/${m.name}`;

      if (typeof capsModel === "object") {
        uiModel.capabilities = extractCapabilities(capsModel as CodeChatModel);
      }
    }

    // Completion-type default
    if (modelType === "completion") {
      uiModel.isDefault = caps.completion_default_model === `refact/${m.name}`;
    }

    return uiModel;
  });
}

/**
 * Group models for UI. Uses default / thinking / light groups when possible.
 * Falls back to a single group if there's no useful structure.
 */
export function groupModelsWithPricing(
  models: SimplifiedModel[],
  options: {
    caps?: CapsResponse;
    modelType: ModelType;
  },
): ModelGroup[] {
  const decorated = attachPricingAndCapabilities(models, options);

  // No caps at all → single group, preserves old UI semantics
  if (!options.caps) {
    return [
      {
        id: "all",
        title:
          options.modelType === "chat" ? "Chat models" : "Completion models",
        models: decorated,
      },
    ];
  }

  const defaultGroup: ModelGroup = {
    id: "default",
    title: "Default",
    description: "Used by default for this provider",
    models: [],
  };
  const thinkingGroup: ModelGroup = {
    id: "thinking",
    title: "Reasoning / Thinking",
    description: "Slower but better at complex reasoning",
    models: [],
  };
  const lightGroup: ModelGroup = {
    id: "light",
    title: "Light / Cheaper",
    description: "Cheaper / faster variants",
    models: [],
  };
  const otherGroup: ModelGroup = {
    id: "other",
    title: "Other models",
    models: [],
  };

  for (const m of decorated) {
    if (m.isDefault) {
      defaultGroup.models.push(m);
    } else if (m.isThinking) {
      thinkingGroup.models.push(m);
    } else if (m.isLight) {
      lightGroup.models.push(m);
    } else {
      otherGroup.models.push(m);
    }
  }

  const groups: ModelGroup[] = [];
  if (defaultGroup.models.length) groups.push(defaultGroup);
  if (thinkingGroup.models.length) groups.push(thinkingGroup);
  if (lightGroup.models.length) groups.push(lightGroup);
  if (otherGroup.models.length) groups.push(otherGroup);

  // If we didn't get any meaningful separation, collapse into a single group
  if (
    groups.length === 1 &&
    groups[0].id === "other" &&
    groups[0].models.length === decorated.length
  ) {
    return [
      {
        id: "all",
        title:
          options.modelType === "chat" ? "Chat models" : "Completion models",
        models: decorated,
      },
    ];
  }

  return groups;
}
