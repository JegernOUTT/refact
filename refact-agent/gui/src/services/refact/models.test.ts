import { describe, expect, test } from "vitest";
import { isModel, isModelsResponse } from "./models";

const CHAT_MODEL = {
  n_ctx: 4096,
  name: "gpt-4o",
  tokenizer: "hf://gpt",
  supports_tools: true,
  supports_multimodality: false,
  supports_clicks: false,
  supports_agent: true,
  default_temperature: null,
  enabled: true,
  type: "chat",
};

describe("models type-guards", () => {
  test("isModelsResponse accepts a response without legacy fields", () => {
    expect(isModelsResponse({ chat_models: [] })).toBe(true);
  });

  test("isModelsResponse tolerates inert legacy completion and embedding fields", () => {
    expect(
      isModelsResponse({
        chat_models: [],
        completion_models: [],
        embedding_model: null,
      }),
    ).toBe(true);
  });

  test("isModelsResponse rejects a response without chat models", () => {
    expect(isModelsResponse({ completion_models: [] })).toBe(false);
    expect(isModelsResponse({ chat_models: "none" })).toBe(false);
    expect(isModelsResponse(null)).toBe(false);
  });

  test("isModel accepts chat models and rejects completion and embedding models", () => {
    expect(isModel(CHAT_MODEL)).toBe(true);
    expect(
      isModel({
        n_ctx: 2048,
        name: "legacy-completion",
        model_family: null,
        type: "completion",
        enabled: true,
      }),
    ).toBe(false);
    expect(
      isModel({
        n_ctx: 512,
        name: "legacy-embedding",
        tokenizer: "fake",
        embedding_size: 1536,
        rejection_threshold: 0.25,
        embedding_batch: 64,
        enabled: true,
        type: "embedding",
      }),
    ).toBe(false);
  });
});
