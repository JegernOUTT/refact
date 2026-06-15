import { describe, expect, test } from "vitest";

import { STUB_CAPS_RESPONSE } from "../../__fixtures__/caps";
import { isCapsResponse } from "./caps";
import { isProviderDefaults, isProviderDetailResponse } from "./providers";

const providerModel = {
  id: "custom/qwen2.5-coder",
  base_name: "qwen2.5-coder",
  enabled: true,
  n_ctx: 131_072,
  supports_tools: false,
  supports_multimodality: false,
  supports_reasoning: null,
  supports_agent: false,
  wire_format_override: null,
  endpoint_override: null,
  user_configured: true,
  removable: true,
};

describe("role-separated model service contracts", () => {
  test("caps parser accepts valid role-separated caps", () => {
    expect(isCapsResponse(STUB_CAPS_RESPONSE)).toBe(true);
    expect(
      STUB_CAPS_RESPONSE.completion_models["openai/qwen2.5/coder/1.5b/base"]
        .completion_endpoint_style,
    ).toBe("openai_completions");
    expect(STUB_CAPS_RESPONSE.embedding_model?.embedding_endpoint_style).toBe(
      "openai",
    );
    expect(
      STUB_CAPS_RESPONSE.embedding_models?.["ollama/nomic-embed-text"]
        .embedding_endpoint_style,
    ).toBe("ollama_native");
  });

  test("caps parser rejects malformed role sections", () => {
    expect(
      isCapsResponse({
        ...STUB_CAPS_RESPONSE,
        completion_models: {
          broken: {
            n_ctx: 4096,
            name: "broken",
            enabled: "yes",
            model_family: null,
            type: "completion",
          },
        },
      }),
    ).toBe(false);

    expect(
      isCapsResponse({
        ...STUB_CAPS_RESPONSE,
        embedding_models: {
          broken: {
            n_ctx: 8191,
            name: "text-embedding-3-small",
            id: "openai/text-embedding-3-small",
            tokenizer: "Xenova/text-embedding-ada-002",
            embedding_size: "1536",
            rejection_threshold: 0.25,
            embedding_batch: 64,
            enabled: true,
            type: "embedding",
          },
        },
      }),
    ).toBe(false);
  });

  test("providers parser round-trips completion and embedding role fields", () => {
    const response = {
      name: "custom",
      base_provider: "custom",
      display_name: "Custom Provider",
      enabled: true,
      readonly: false,
      has_credentials: true,
      selected_models_count: 3,
      status: "active",
      settings: {
        wire_format: "openai_chat_completions",
        completion_endpoint_style: "openai_chat_completions",
        embedding_endpoint_style: "ollama_native",
        completion_models: {
          "qwen2.5-coder": { n_ctx: 131_072, model_family: "qwen2.5-coder" },
        },
        embedding_model: {
          name: "nomic-embed-text",
          embedding_size: 768,
        },
      },
      runtime: {
        name: "custom",
        base_provider: "custom",
        display_name: "Custom Provider",
        enabled: true,
        readonly: false,
        wire_format: "openai_chat_completions",
        completion_endpoint_style: "openai_chat_completions",
        embedding_endpoint_style: "ollama_native",
        chat_endpoint: "https://api.example.test/v1/chat/completions",
        completion_endpoint: "https://api.example.test/v1/chat/completions",
        embedding_endpoint: "https://api.example.test/v1/embeddings",
        chat_models: [providerModel],
        completion_models: [providerModel],
        embedding_model: {
          ...providerModel,
          id: "custom/nomic-embed-text",
          base_name: "nomic-embed-text",
        },
      },
    };

    expect(isProviderDetailResponse(response)).toBe(true);
  });

  test("providers parser rejects malformed role endpoint styles", () => {
    expect(
      isProviderDetailResponse({
        name: "custom",
        base_provider: "custom",
        display_name: "Custom Provider",
        enabled: true,
        readonly: false,
        has_credentials: true,
        selected_models_count: 1,
        status: "active",
        settings: {},
        runtime: {
          name: "custom",
          display_name: "Custom Provider",
          enabled: true,
          readonly: false,
          wire_format: "openai_chat_completions",
          completion_endpoint_style: "anthropic_messages",
          embedding_endpoint_style: "openai",
          chat_endpoint: "https://api.example.test/v1/chat/completions",
          completion_endpoint: "https://api.example.test/v1/completions",
          embedding_endpoint: "https://api.example.test/v1/embeddings",
          chat_models: [],
          completion_models: [],
          embedding_model: null,
        },
      }),
    ).toBe(false);
  });

  test("providers parser validates role-separated settings shape", () => {
    const base = {
      name: "custom",
      base_provider: "custom",
      display_name: "Custom Provider",
      enabled: true,
      readonly: false,
      has_credentials: true,
      selected_models_count: 1,
      status: "active",
    };

    // completion_models must be an object map, not a bare string
    expect(
      isProviderDetailResponse({
        ...base,
        settings: { completion_models: "qwen2.5-coder" },
      }),
    ).toBe(false);

    // embedding_model must be a string or object, not a number
    expect(
      isProviderDetailResponse({
        ...base,
        settings: { embedding_model: 7 },
      }),
    ).toBe(false);

    // invalid completion_endpoint_style in settings is rejected
    expect(
      isProviderDetailResponse({
        ...base,
        settings: { completion_endpoint_style: "anthropic_messages" },
      }),
    ).toBe(false);

    // legacy bare-string embedding_model shape is accepted for back-compat
    expect(
      isProviderDetailResponse({
        ...base,
        settings: { embedding_model: "custom/nomic-embed-text" },
      }),
    ).toBe(true);

    // a well-formed role-separated settings object passes
    expect(
      isProviderDetailResponse({
        ...base,
        settings: {
          completion_endpoint_style: "openai_completions",
          completion_models: { "qwen2.5-coder": { n_ctx: 8192 } },
          embedding_endpoint_style: "openai",
          embedding_model: { name: "nomic-embed-text", embedding_size: 768 },
        },
      }),
    ).toBe(true);
  });

  test("defaults parser validates completion_model and embedding_model", () => {
    expect(
      isProviderDefaults({
        chat: { model: "openai/gpt-4.1", temperature: 0.2 },
        chat_model_2: {},
        task_planner_agent_model: {},
        chat_light: {},
        chat_thinking: {},
        chat_buddy: {},
        completion_model: "custom/qwen2.5-coder",
        embedding_model: null,
      }),
    ).toBe(true);

    expect(
      isProviderDefaults({
        chat: {},
        completion_model: 7,
        embedding_model: "custom/nomic-embed-text",
      }),
    ).toBe(false);

    expect(
      isProviderDefaults({
        chat: {},
        completion_model: "custom/qwen2.5-coder",
        embedding_model: ["custom/nomic-embed-text"],
      }),
    ).toBe(false);
  });
});
