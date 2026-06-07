import { describe, expect, test, beforeEach } from "vitest";
import { http, HttpResponse } from "msw";

import { STUB_CAPS_RESPONSE } from "../../__fixtures__/caps";
import type { CapsResponse } from "../../services/refact";
import { createDefaultChatState, render, screen } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { CapsSelect } from "./ChatControls";

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
const goodChatModes = http.get("*/v1/chat-modes", () =>
  HttpResponse.json({ modes: [], errors: [] }),
);
const queuedChatCommand = http.post("*/v1/chats/:id/commands", () =>
  HttpResponse.json({ status: "queued" }),
);

function capsWithUnavailableModel(): CapsResponse {
  const caps = structuredClone(STUB_CAPS_RESPONSE);
  return {
    ...caps,
    chat_default_model: "openai/gpt-4o",
    chat_model_2: "openai/gpt-4o-mini",
    task_planner_agent_model: "openai/o3-mini",
    chat_thinking_model: "openai/o1",
    chat_light_model: "openai/o1-mini",
    chat_buddy_model: "openai/claude-3-5-haiku",
    chat_models: {
      ...caps.chat_models,
      "disabled/model": {
        n_ctx: 32000,
        name: "disabled/model",
        id: "disabled/model",
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
    metadata: {
      pricing: {
        "openai/gpt-4o": { prompt: 2.5, generated: 10, cache_read: 1.25 },
        "openai/gpt-4o-mini": { prompt: 0.15, generated: 0.6 },
      },
    },
  };
}

function chatState(model = "openai/gpt-4o") {
  const chat = createDefaultChatState();
  const runtime = chat.threads[chat.current_thread_id];
  runtime.thread.model = model;
  return chat;
}

function mockCaps(caps: CapsResponse) {
  server.use(
    goodPing,
    goodChatModes,
    queuedChatCommand,
    http.get("*/v1/caps", () => HttpResponse.json(caps)),
  );
}

describe("CapsSelect", () => {
  beforeEach(() => {
    mockCaps(capsWithUnavailableModel());
  });

  test("renders enriched model options with badges, pricing, context window, and add-new navigation", async () => {
    const { user, store } = render(<CapsSelect />, {
      preloadedState: { chat: chatState(), config },
    });

    await user.click(await screen.findByRole("combobox", { name: "chat model" }));

    expect(
      await screen.findByRole("option", {
        name: /openai\/gpt-4o.*Default.*\$2\.50\/\$10\.00.*128K/s,
      }),
    ).toBeInTheDocument();
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

    await user.click(screen.getByRole("option", { name: "Add new model" }));
    expect(store.getState().pages.at(-1)).toEqual({ name: "providers page" });
  });

  test("selected model changes update thread model and context window limits", async () => {
    const { user, store } = render(<CapsSelect />, {
      preloadedState: { chat: chatState("openai/o1"), config },
    });

    await user.click(await screen.findByRole("combobox", { name: "chat model" }));
    await user.click(
      await screen.findByRole("option", { name: /openai\/gpt-4o-mini/ }),
    );

    const thread = store.getState().chat.threads[store.getState().chat.current_thread_id]?.thread;
    expect(thread?.model).toBe("openai/gpt-4o-mini");
    expect(thread?.modelMaximumContextTokens).toBe(128000);
    expect(thread?.currentMaximumContextTokens).toBe(128000);
  });

  test.skip("TODO disabled models from the caps pipeline cannot be selected", () => {
    expect(true).toBe(true);
  });
});
