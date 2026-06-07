import { describe, expect, test, beforeEach, vi } from "vitest";
import { http, HttpResponse } from "msw";

import { STUB_CAPS_RESPONSE } from "../../__fixtures__/caps";
import type { CapsResponse } from "../../services/refact";
import { server } from "../../utils/mockServer";
import { render, screen, within } from "../../utils/test-utils";
import { enrichAndGroupModels } from "../../utils/enrichModels";
import { ModelSelector } from "./ModelSelector";

HTMLElement.prototype.hasPointerCapture = () => false;
HTMLElement.prototype.releasePointerCapture = () => undefined;

const config = {
  apiKey: "test",
  host: "web" as const,
  dev: true,
  themeProps: {},
  lspPort: 8001,
};

const goodPing = http.get("*/v1/ping", () => HttpResponse.text("pong"));

function capsWithModelRoles(): CapsResponse {
  return {
    ...structuredClone(STUB_CAPS_RESPONSE),
    chat_default_model: "openai/gpt-4o",
    task_planner_agent_model: "openai/o3-mini",
    chat_model_2: "openai/gpt-4o-mini",
    chat_thinking_model: "openai/o1",
    chat_light_model: "openai/o1-mini",
    chat_buddy_model: "openai/claude-3-5-haiku",
    metadata: {
      pricing: {
        "openai/gpt-4o": { prompt: 2.5, generated: 10, cache_read: 1.25 },
        "openai/o3-mini": { prompt: 1.1, generated: 4.4 },
      },
    },
    chat_models: {
      ...structuredClone(STUB_CAPS_RESPONSE.chat_models),
      "refact/legacy-model": {
        n_ctx: 8192,
        name: "legacy-model",
        id: "refact/legacy-model",
        type: "chat",
        enabled: true,
        tokenizer: "fake",
        supports_tools: true,
        supports_multimodality: false,
        supports_clicks: false,
        supports_agent: false,
        reasoning_effort_options: null,
        supports_thinking_budget: false,
        default_temperature: null,
      },
    },
  };
}

function mockCaps(caps: CapsResponse) {
  server.use(
    goodPing,
    http.get("*/v1/caps", () => HttpResponse.json(caps)),
  );
}

describe("model selector caps pipeline", () => {
  beforeEach(() => {
    mockCaps(capsWithModelRoles());
  });

  test("enrichAndGroupModels filters usable models, groups by provider, sorts role models first, and enriches metadata", () => {
    const caps = capsWithModelRoles();
    const usableModels = Object.keys(caps.chat_models)
      .filter((model) => !model.startsWith("refact/"))
      .map((model) => ({ value: model, textValue: model, disabled: false }));

    const groups = enrichAndGroupModels(usableModels, caps);

    expect(groups.map((group) => group.displayName).slice(0, 3)).toEqual([
      "OpenAI",
      "Anthropic",
      "Google",
    ]);
    expect(groups.some((group) => group.models.some((model) => model.value === "refact/legacy-model"))).toBe(false);

    const openAIModels = groups.find((group) => group.provider === "openai")?.models;
    expect(openAIModels?.map((model) => model.value).slice(0, 5)).toEqual([
      "openai/gpt-4o",
      "openai/o3-mini",
      "openai/gpt-4o-mini",
      "openai/o1",
      "openai/o1-mini",
    ]);

    expect(openAIModels?.[0]).toMatchObject({
      value: "openai/gpt-4o",
      isDefault: true,
      pricing: { prompt: 2.5, generated: 10, cache_read: 1.25 },
      nCtx: 128000,
      capabilities: {
        supportsTools: true,
        supportsMultimodality: true,
        supportsAgent: true,
      },
    });
    expect(openAIModels?.[1]).toMatchObject({
      value: "openai/o3-mini",
      isTaskPlannerAgent: true,
    });
  });

  test("renders badges, pricing, context window, unavailable selection, and allowUnset behavior", async () => {
    const onValueChange = vi.fn();

    const { user } = render(
      <ModelSelector
        value="missing/model"
        onValueChange={onValueChange}
        compact={false}
        allowUnset
      />,
      { preloadedState: { config } },
    );

    await user.click(await screen.findByRole("combobox"));

    expect(
      await screen.findByRole("option", { name: /Unavailable: missing\/model/ }),
    ).toHaveAttribute("aria-disabled", "true");

    await user.click(screen.getByRole("option", { name: /None None/ }));
    expect(onValueChange).toHaveBeenCalledWith("");

    await user.click(screen.getByRole("combobox"));
    expect(screen.queryByRole("option", { name: /refact\/legacy-model/ })).not.toBeInTheDocument();

    const defaultOption = await screen.findByRole("option", {
      name: /openai\/gpt-4o.*Default.*\$2\.50\/\$10\.00.*128K/s,
    });
    expect(defaultOption).toBeInTheDocument();
    expect(
      screen.getByRole("option", { name: /openai\/o3-mini.*Task Agent/s }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("option", { name: /openai\/gpt-4o-mini.*Chat 2/s }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("option", { name: /openai\/o1.*Reasoning/s }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("option", { name: /openai\/o1-mini.*Light/s }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("option", {
        name: /openai\/claude-3-5-haiku.*Companion/s,
      }),
    ).toBeInTheDocument();
  });

  test("selecting an enabled model calls onValueChange, while unavailable selected model is disabled", async () => {
    const onValueChange = vi.fn();

    const { user } = render(
      <ModelSelector
        value="missing/model"
        onValueChange={onValueChange}
        compact={false}
      />,
      { preloadedState: { config } },
    );

    await user.click(await screen.findByRole("combobox"));

    const listbox = await screen.findByRole("listbox");
    expect(
      within(listbox).getByRole("option", { name: /Unavailable: missing\/model/ }),
    ).toHaveAttribute("aria-disabled", "true");

    await user.click(
      within(listbox).getByRole("option", { name: /openai\/gpt-4o-mini/ }),
    );

    expect(onValueChange).toHaveBeenCalledWith("openai/gpt-4o-mini");
  });
});
