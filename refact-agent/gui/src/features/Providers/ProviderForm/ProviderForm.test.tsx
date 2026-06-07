import { describe, expect, it } from "vitest";

import { normalizeProviderDefaults } from "./ProviderForm";
import type { ProviderDefaults } from "../../../services/refact";

describe("normalizeProviderDefaults", () => {
  it("preserves unknown default extension fields", () => {
    const defaults = {
      chat: { model: "openai/gpt-4.1" },
      chat_model_2: {},
      task_planner_agent_model: {},
      chat_light: {},
      chat_thinking: {},
      completion_model: "qwen-coder",
      embedding_model: "nomic-embed",
      future_default_role: { model: "future/model" },
    } satisfies ProviderDefaults & {
      future_default_role: { model: string };
    };

    expect(normalizeProviderDefaults(defaults)).toEqual({
      chat: { model: "openai/gpt-4.1" },
      chat_model_2: {},
      task_planner_agent_model: {},
      chat_light: {},
      chat_thinking: {},
      chat_buddy: {},
      completion_model: "qwen-coder",
      embedding_model: "nomic-embed",
      future_default_role: { model: "future/model" },
    });
  });
});
