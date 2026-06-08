import { describe, expect, test, beforeEach } from "vitest";
import { http, HttpResponse } from "msw";
import { screen } from "../../utils/test-utils";
import { render } from "../../utils/test-utils";
import { server, goodCaps } from "../../utils/mockServer";
import { createDefaultChatState } from "../../utils/test-utils";
import { ChatSettingsDropdown } from "./ChatSettingsDropdown";

function chatStateWithReasoning(enabled: boolean) {
  const chat = createDefaultChatState();
  const threadId = chat.current_thread_id;
  const runtime = chat.threads[threadId];
  runtime.thread.model = "openai/o1";
  runtime.thread.boost_reasoning = enabled;
  runtime.thread.reasoning_effort = "high";
  runtime.thread.thinking_budget = 4096;
  runtime.thread.temperature = 0.7;
  return chat;
}

const config = {
  apiKey: "test",
  host: "web" as const,
  dev: true,
  themeProps: {},
  lspPort: 8001,
};

const modes = [
  {
    id: "agent",
    title: "Agent",
    description: "Autonomous coding mode",
    tools_count: 12,
    thread_defaults: {
      include_project_info: true,
      checkpoints_enabled: true,
      auto_approve_editing_tools: false,
      auto_approve_dangerous_commands: false,
    },
    ui: { order: 1, tags: ["editing", "tools"] },
  },
];

const goodChatModes = http.get("*/v1/chat-modes", () =>
  HttpResponse.json({ modes, errors: [] }),
);

const goodPing = http.get("*/v1/ping", () => HttpResponse.text("pong"));

const queuedChatCommand = http.post("*/v1/chats/:id/commands", () =>
  HttpResponse.json({ status: "queued" }),
);

describe("ChatSettingsDropdown", () => {
  beforeEach(() => {
    server.use(goodCaps, goodChatModes, goodPing, queuedChatCommand);
  });

  test("turning reasoning on clears temperature", async () => {
    const { user, store } = render(<ChatSettingsDropdown />, {
      preloadedState: {
        chat: chatStateWithReasoning(false),
        config,
      },
    });

    await user.click(await screen.findByRole("button", { name: /openai\/o1/ }));
    await user.click(await screen.findByRole("switch"));

    const thread = store.getState().chat.threads[store.getState().chat.current_thread_id]?.thread;
    expect(thread?.boost_reasoning).toBe(true);
    expect(thread?.temperature).toBeNull();
  });

  test("turning reasoning off clears reasoning effort and thinking budget", async () => {
    const { user, store } = render(<ChatSettingsDropdown />, {
      preloadedState: {
        chat: chatStateWithReasoning(true),
        config,
      },
    });

    await user.click(await screen.findByRole("button", { name: /openai\/o1/ }));
    await user.click(await screen.findByRole("switch"));

    const thread = store.getState().chat.threads[store.getState().chat.current_thread_id]?.thread;
    expect(thread?.boost_reasoning).toBe(false);
    expect(thread?.reasoning_effort).toBeNull();
    expect(thread?.thinking_budget).toBeNull();
  });

});
