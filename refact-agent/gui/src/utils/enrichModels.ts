import type { CapCost, CapsResponse } from "../services/refact";
import type { ModelCapabilities } from "../features/Providers/ProviderForm/ProviderModelsList/utils/groupModelsWithPricing";
import { extractProvider, getProviderDisplayName } from "./modelProviders";

export type ModelSelectorCapability = "chat" | "completion" | "embedding";

export type EnrichedModel = {
  value: string;
  displayName: string;
  disabled: boolean;
  pricing?: CapCost;
  nCtx?: number;
  capabilities?: ModelCapabilities;
  metadata?: string[];
  isDefault?: boolean;
  isChat2?: boolean;
  isTaskPlannerAgent?: boolean;
  isThinking?: boolean;
  isLight?: boolean;
  isBuddy?: boolean;
  provider: string;
};

export type ModelGroup = {
  provider: string;
  displayName: string;
  models: EnrichedModel[];
};

type UsableModel = {
  value: string;
  textValue: string;
  disabled: boolean;
};

const PROVIDER_PRIORITY: Record<string, number> = {
  openai: 1,
  anthropic: 2,
  google: 3,
  "x.ai": 4,
  meta: 5,
  mistral: 6,
};

export function pricingForModel(
  pricing: Record<string, CapCost | undefined> | undefined,
  modelKey: string,
  displayName: string,
): CapCost | undefined {
  if (!pricing) return undefined;
  return (
    pricing[modelKey] ??
    pricing[displayName] ??
    pricing[modelKey.replace(/^refact\//, "")]
  );
}

function extractCapabilities(
  capsModel: CapsResponse["chat_models"][string] | undefined,
): ModelCapabilities | undefined {
  if (capsModel === undefined) return undefined;

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

function getPricing(
  modelKey: string,
  displayName: string,
  caps: CapsResponse,
): CapCost | undefined {
  return pricingForModel(caps.metadata?.pricing, modelKey, displayName);
}

function getContextWindow(
  capsModel:
    | CapsResponse["chat_models"][string]
    | CapsResponse["completion_models"][string]
    | CapsResponse["embedding_model"]
    | undefined,
): number | undefined {
  if (capsModel === undefined) return undefined;
  return capsModel.n_ctx;
}

function completionMetadata(
  capsModel: CapsResponse["completion_models"][string] | undefined,
): string[] | undefined {
  if (!capsModel) return undefined;
  return [capsModel.model_family, capsModel.name]
    .filter((value): value is string => Boolean(value))
    .filter((value, index, values) => values.indexOf(value) === index);
}

function embeddingMetadata(
  capsModel: CapsResponse["embedding_model"] | undefined,
): string[] | undefined {
  if (!capsModel) return undefined;
  return [
    `${capsModel.embedding_size} dims`,
    `batch ${capsModel.embedding_batch}`,
    `threshold ${capsModel.rejection_threshold}`,
  ];
}

export function enrichModels(
  usableModels: UsableModel[],
  caps: CapsResponse | undefined,
  capability: ModelSelectorCapability = "chat",
): EnrichedModel[] {
  if (!caps) {
    return usableModels.map((model) => ({
      value: model.value,
      displayName: model.textValue,
      disabled: model.disabled,
      provider: extractProvider(model.value),
    }));
  }

  return usableModels.map((model) => {
    const modelKey = model.value;
    const displayName = model.textValue;

    if (capability === "completion") {
      const capsModel = caps.completion_models[modelKey];
      return {
        value: modelKey,
        displayName,
        disabled: model.disabled,
        pricing: getPricing(modelKey, displayName, caps),
        nCtx: getContextWindow(capsModel),
        metadata: completionMetadata(capsModel),
        isDefault: caps.completion_default_model === modelKey,
        provider: extractProvider(modelKey),
      };
    }

    if (capability === "embedding") {
      const capsModel =
        caps.embedding_model?.id === modelKey
          ? caps.embedding_model
          : undefined;
      return {
        value: modelKey,
        displayName,
        disabled: model.disabled,
        nCtx: getContextWindow(capsModel),
        metadata: embeddingMetadata(capsModel),
        isDefault: caps.embedding_model?.id === modelKey,
        provider: extractProvider(modelKey),
      };
    }

    const capsModel = caps.chat_models[modelKey];
    return {
      value: modelKey,
      displayName,
      disabled: model.disabled,
      pricing: getPricing(modelKey, displayName, caps),
      nCtx: getContextWindow(capsModel),
      capabilities: extractCapabilities(capsModel),
      isDefault: caps.chat_default_model === modelKey,
      isChat2: caps.chat_model_2 === modelKey,
      isTaskPlannerAgent: caps.task_planner_agent_model === modelKey,
      isThinking: caps.chat_thinking_model === modelKey,
      isLight: caps.chat_light_model === modelKey,
      isBuddy: caps.chat_buddy_model === modelKey,
      provider: extractProvider(modelKey),
    };
  });
}

function sortModelsInGroup(models: EnrichedModel[]): EnrichedModel[] {
  return [...models].sort((a, b) => {
    if (a.isDefault) return -1;
    if (b.isDefault) return 1;
    if (a.isTaskPlannerAgent) return -1;
    if (b.isTaskPlannerAgent) return 1;
    if (a.isChat2) return -1;
    if (b.isChat2) return 1;
    if (a.isThinking) return -1;
    if (b.isThinking) return 1;
    if (a.isLight) return -1;
    if (b.isLight) return 1;
    if (a.isBuddy) return -1;
    if (b.isBuddy) return 1;
    return a.displayName.localeCompare(b.displayName);
  });
}

export function groupModelsByProvider(models: EnrichedModel[]): ModelGroup[] {
  const groups = models.reduce<Record<string, EnrichedModel[]>>(
    (acc, model) => {
      if (Object.prototype.hasOwnProperty.call(acc, model.provider)) {
        acc[model.provider].push(model);
      } else {
        acc[model.provider] = [model];
      }

      return acc;
    },
    {},
  );

  return Object.entries(groups)
    .map(([provider, providerModels]) => ({
      provider,
      displayName: getProviderDisplayName(provider),
      models: sortModelsInGroup(providerModels),
    }))
    .sort((a, b) => {
      const aPri = PROVIDER_PRIORITY[a.provider] || 999;
      const bPri = PROVIDER_PRIORITY[b.provider] || 999;
      if (aPri !== bPri) return aPri - bPri;
      return a.displayName.localeCompare(b.displayName);
    });
}

export function enrichAndGroupModels(
  usableModels: UsableModel[],
  caps: CapsResponse | undefined,
  capability: ModelSelectorCapability = "chat",
): ModelGroup[] {
  const enriched = enrichModels(usableModels, caps, capability);
  return groupModelsByProvider(enriched);
}
