import { describe, expect, it } from "vitest";
import type {
  CapsResponse,
  CodeChatModel,
  SimplifiedModel,
} from "../../../../../services/refact";
import {
  attachPricingAndCapabilities,
  groupModelsWithPricing,
} from "./groupModelsWithPricing";

function chatModel(id: string, name: string): CodeChatModel {
  return {
    id,
    name,
    n_ctx: 128000,
    tokenizer: "fake",
    supports_tools: true,
    supports_multimodality: false,
    supports_clicks: false,
    supports_agent: true,
    default_temperature: null,
    enabled: true,
    type: "chat",
  };
}

function capsWithOpenAiModel(): CapsResponse {
  return {
    caps_version: 1,
    chat_default_model: "openai/gpt-4.1",
    chat_model_2: "",
    task_planner_agent_model: "openai/gpt-4.1",
    chat_thinking_model: "",
    chat_light_model: "",
    chat_buddy_model: "",
    chat_models: {
      "openai/gpt-4.1": chatModel("openai/gpt-4.1", "gpt-4.1"),
    },
    code_chat_default_system_prompt: "",
    completion_models: {},
    completion_default_model: "",
    code_completion_n_ctx: 0,
    endpoint_chat_passthrough: "",
    endpoint_style: "",
    endpoint_template: "",
    running_models: [],
    tokenizer_path_template: "",
    tokenizer_rewrite_path: {},
    metadata: {
      pricing: {
        "gpt-4.1": {
          prompt: 2,
          generated: 8,
        },
      },
    },
    customization: "",
  };
}

describe("attachPricingAndCapabilities", () => {
  it("matches provider-qualified caps models for bare provider rows", () => {
    const models: SimplifiedModel[] = [
      {
        name: "gpt-4.1",
        enabled: true,
        removable: false,
        user_configured: false,
      },
    ];

    const [model] = attachPricingAndCapabilities(models, {
      caps: capsWithOpenAiModel(),
      modelType: "chat",
      providerName: "openai",
    });

    expect(model.nCtx).toBe(128000);
    expect(model.capabilities?.supportsTools).toBe(true);
    expect(model.pricing?.prompt).toBe(2);
    expect(model.isDefault).toBe(true);
    expect(model.isTaskPlannerAgent).toBe(true);
  });

  it("prefers current provider when duplicate bare model names exist", () => {
    const caps = capsWithOpenAiModel();
    caps.chat_default_model = "openai_2/gpt-4.1";
    caps.task_planner_agent_model = "";
    caps.chat_models["openai_2/gpt-4.1"] = {
      ...chatModel("openai_2/gpt-4.1", "gpt-4.1"),
      n_ctx: 64000,
      supports_tools: false,
    };
    caps.metadata = {
      pricing: {
        "openai/gpt-4.1": { prompt: 2, generated: 8 },
        "openai_2/gpt-4.1": { prompt: 1, generated: 4 },
      },
    };

    const [model] = attachPricingAndCapabilities(
      [
        {
          name: "gpt-4.1",
          enabled: true,
          removable: false,
          user_configured: false,
        },
      ],
      {
        caps,
        modelType: "chat",
        providerName: "openai_2",
      },
    );

    expect(model.nCtx).toBe(64000);
    expect(model.capabilities?.supportsTools).toBe(false);
    expect(model.pricing?.prompt).toBe(1);
    expect(model.isDefault).toBe(true);
    expect(model.isTaskPlannerAgent).toBe(false);
  });

  it("labels embedding-only groups distinctly", () => {
    const groups = groupModelsWithPricing(
      [
        {
          name: "openai/thenlper/gte-base",
          enabled: true,
          removable: false,
          user_configured: false,
        },
      ],
      {
        caps: capsWithOpenAiModel(),
        modelType: "embedding",
        providerName: "openai",
      },
    );

    expect(groups[0].title).toBe("Embedding models");
    expect(groups[0].models[0].modelType).toBe("embedding");
  });
});
