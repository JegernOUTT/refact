import { describe, expect, test, beforeEach } from "vitest";
import { http, HttpResponse } from "msw";
import { fireEvent, screen, waitFor } from "../../utils/test-utils";
import { applyChatEvent } from "../../features/Chat/Thread/actions";
import { render } from "../../utils/test-utils";
import { server, goodCaps } from "../../utils/mockServer";
import { createDefaultChatState } from "../../utils/test-utils";
import { ChatSettingsDropdown } from "./ChatSettingsDropdown";
import { ChatThreadProvider } from "../../features/Chat/Thread";
import { STUB_CAPS_RESPONSE } from "../../__fixtures__/caps";

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

    const thread =
      store.getState().chat.threads[store.getState().chat.current_thread_id]
        ?.thread;
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

    const thread =
      store.getState().chat.threads[store.getState().chat.current_thread_id]
        ?.thread;
    expect(thread?.boost_reasoning).toBe(false);
    expect(thread?.reasoning_effort).toBeNull();
    expect(thread?.thinking_budget).toBeNull();
  });

  test("selected model change only updates the scoped thread", async () => {
    const chat = chatStateWithReasoning(false);
    const currentId = chat.current_thread_id;
    const otherId = "thread-b";
    const otherRuntime = structuredClone(chat.threads[currentId]);
    otherRuntime.thread.id = otherId;
    otherRuntime.thread.model = "openai/gpt-4o";
    chat.open_thread_ids.push(otherId);
    chat.threads[otherId] = otherRuntime;

    const { user, store } = render(
      <ChatThreadProvider chatId={otherId}>
        <ChatSettingsDropdown />
      </ChatThreadProvider>,
      {
        preloadedState: {
          chat,
          config,
        },
      },
    );

    await user.click(
      await screen.findByRole("button", { name: /openai\/gpt-4o/ }),
    );
    await user.click(
      await screen.findByRole("option", { name: /openai\/gpt-4o-mini/ }),
    );

    const state = store.getState();
    expect(state.chat.threads[currentId]?.thread.model).toBe("openai/o1");
    expect(state.chat.threads[otherId]?.thread.model).toBe(
      "openai/gpt-4o-mini",
    );
  });

  test("shows the 90% default and resets auto-compression cap", async () => {
    const chat = chatStateWithReasoning(false);
    const runtime = chat.threads[chat.current_thread_id];
    runtime.thread.auto_compression_cap = 8192;
    const { user, store } = render(<ChatSettingsDropdown />, {
      preloadedState: { chat, config },
    });

    await user.click(await screen.findByRole("button", { name: /openai\/o1/ }));
    await user.click(screen.getByRole("button", { name: /Token limits/ }));
    expect(screen.getByText("Auto-compression cap")).toBeInTheDocument();
    expect(
      screen.getByText(/90% of the effective model\/request/i),
    ).toBeInTheDocument();
    expect(screen.getAllByText("200K").length).toBeGreaterThan(0);
    expect(
      store.getState().chat.threads[chat.current_thread_id]?.thread
        .auto_compression_cap,
    ).toBe(8192);

    await user.click(
      screen.getByRole("button", { name: "Reset auto-compression cap" }),
    );
    expect(
      store.getState().chat.threads[chat.current_thread_id]?.thread
        .auto_compression_cap,
    ).toBe(180000);
    expect(screen.getByText("180000")).toBeInTheDocument();
  });

  test("allows auto-compression cap changes while waiting but not while streaming", async () => {
    const chat = chatStateWithReasoning(false);
    const runtime = chat.threads[chat.current_thread_id];
    runtime.thread.auto_compression_cap = 8192;
    runtime.waiting_for_response = true;
    const { user, store } = render(<ChatSettingsDropdown />, {
      preloadedState: { chat, config },
    });

    const trigger = await screen.findByRole("button", { name: /openai\/o1/ });
    expect(trigger).toBeEnabled();
    await user.click(trigger);
    await user.click(screen.getByRole("button", { name: /Token limits/ }));
    const reset = screen.getByRole("button", {
      name: "Reset auto-compression cap",
    });
    expect(reset).toBeEnabled();
    await user.click(reset);
    expect(
      store.getState().chat.threads[chat.current_thread_id]?.thread
        .auto_compression_cap,
    ).toBe(180000);

    await user.keyboard("{Escape}");
    store.dispatch(
      applyChatEvent({
        chat_id: chat.current_thread_id,
        seq: "1",
        type: "runtime_updated",
        state: "generating",
      }),
    );
    await waitFor(() =>
      expect(screen.getByRole("button", { name: /openai\/o1/ })).toBeDisabled(),
    );
  });

  test("sets and resets the auto-compression cap from the token limits disclosure", async () => {
    const chat = chatStateWithReasoning(false);
    const { user, store } = render(<ChatSettingsDropdown />, {
      preloadedState: { chat, config },
    });

    await user.click(await screen.findByRole("button", { name: /openai\/o1/ }));
    await user.click(screen.getByRole("button", { name: /Token limits/ }));

    // Restored unset caps remain uncapped until the user explicitly resets.
    expect(
      store.getState().chat.threads[chat.current_thread_id]?.thread
        .auto_compression_cap,
    ).toBeUndefined();
    expect(screen.getByText("200K (no cap)")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Reset auto-compression cap" }),
    ).toBeInTheDocument();

    fireEvent.keyDown(
      screen.getByRole("slider", { name: "Auto-compression cap" }),
      { key: "ArrowLeft" },
    );

    const cap =
      store.getState().chat.threads[chat.current_thread_id]?.thread
        .auto_compression_cap;
    expect(typeof cap).toBe("number");
    expect(cap).toBeLessThan(200000);

    await user.click(
      screen.getByRole("button", { name: "Reset auto-compression cap" }),
    );
    expect(
      store.getState().chat.threads[chat.current_thread_id]?.thread
        .auto_compression_cap,
    ).toBe(180000);
    expect(screen.getByText("180000")).toBeInTheDocument();
  });

  test.each([
    [512, 460],
    [100001, 90000],
    [300000, 180000],
    [0, 180000],
    [-1, 180000],
  ])(
    "reset respects positive request cap %s and floors 90%% to %s",
    async (requestCap, expectedCap) => {
      const chat = chatStateWithReasoning(false);
      const thread = chat.threads[chat.current_thread_id].thread;
      thread.context_tokens_cap = requestCap;
      thread.auto_compression_cap = 7777;
      const { user, store } = render(<ChatSettingsDropdown />, {
        preloadedState: { chat, config },
      });
      await user.click(
        await screen.findByRole("button", { name: /openai\/o1/ }),
      );
      await user.click(screen.getByRole("button", { name: /Token limits/ }));
      expect(screen.getByText("7777")).toBeInTheDocument();
      expect(
        screen.getByRole("slider", { name: "Auto-compression cap" }),
      ).toHaveAttribute(
        "aria-valuemax",
        String(requestCap > 0 ? Math.min(requestCap, 200000) : 200000),
      );
      expect(
        store.getState().chat.threads[chat.current_thread_id]?.thread
          .auto_compression_cap,
      ).toBe(7777);
      await user.click(
        screen.getByRole("button", { name: "Reset auto-compression cap" }),
      );
      expect(
        store.getState().chat.threads[chat.current_thread_id]?.thread
          .auto_compression_cap,
      ).toBe(expectedCap);
    },
  );

  test("does not change an explicit cap when selecting a smaller model", async () => {
    const chat = chatStateWithReasoning(false);
    chat.threads[chat.current_thread_id].thread.auto_compression_cap = 190000;
    const { user, store } = render(<ChatSettingsDropdown />, {
      preloadedState: { chat, config },
    });
    await user.click(await screen.findByRole("button", { name: /openai\/o1/ }));
    await user.click(
      await screen.findByRole("option", { name: /openai\/gpt-4o-mini/ }),
    );
    expect(
      store.getState().chat.threads[chat.current_thread_id]?.thread
        .auto_compression_cap,
    ).toBe(190000);
    await user.click(screen.getByRole("button", { name: /Token limits/ }));
    expect(screen.getByText("190000")).toBeInTheDocument();
  });

  test.each(["clear", "raise"])(
    "can %s a saved 200K request limit on a 1M model without changing compression",
    async (operation) => {
      server.use(
        http.get("*/v1/caps", () =>
          HttpResponse.json({
            ...STUB_CAPS_RESPONSE,
            chat_models: {
              ...STUB_CAPS_RESPONSE.chat_models,
              "openai/o1": {
                ...STUB_CAPS_RESPONSE.chat_models["openai/o1"],
                n_ctx: 1000000,
              },
            },
          }),
        ),
      );
      const chat = chatStateWithReasoning(false);
      const threadId = chat.current_thread_id;
      chat.threads[threadId].thread.context_tokens_cap = 200000;
      chat.threads[threadId].thread.auto_compression_cap = 180000;
      const { user, store } = render(<ChatSettingsDropdown />, {
        preloadedState: { chat, config },
      });
      await user.click(
        await screen.findByRole("button", { name: /openai\/o1/ }),
      );
      await user.click(screen.getByRole("button", { name: /Token limits/ }));
      expect(
        screen.getByRole("slider", { name: "Request context limit" }),
      ).toHaveAttribute("aria-valuemax", "1000000");
      expect(
        screen.getByRole("slider", { name: "Auto-compression cap" }),
      ).toHaveAttribute("aria-valuemax", "200000");

      if (operation === "clear") {
        await user.click(
          screen.getByRole("button", { name: "Reset request context limit" }),
        );
      } else {
        fireEvent.keyDown(
          screen.getByRole("slider", { name: "Request context limit" }),
          { key: "End" },
        );
      }

      expect(
        store.getState().chat.threads[threadId]?.thread.context_tokens_cap,
      ).toBe(operation === "clear" ? undefined : 1000000);
      expect(
        store.getState().chat.threads[threadId]?.thread.auto_compression_cap,
      ).toBe(180000);
      expect(
        screen.getByRole("slider", { name: "Auto-compression cap" }),
      ).toHaveAttribute("aria-valuemax", "1000000");
      await user.click(
        screen.getByRole("button", { name: "Reset auto-compression cap" }),
      );
      expect(
        store.getState().chat.threads[threadId]?.thread.auto_compression_cap,
      ).toBe(900000);
    },
  );

  test("server request-limit clear updates the open controls", async () => {
    const chat = chatStateWithReasoning(false);
    const threadId = chat.current_thread_id;
    chat.threads[threadId].thread.context_tokens_cap = 100000;
    chat.threads[threadId].thread.auto_compression_cap = 7777;
    const { user, store } = render(<ChatSettingsDropdown />, {
      preloadedState: { chat, config },
    });
    await user.click(await screen.findByRole("button", { name: /openai\/o1/ }));
    await user.click(screen.getByRole("button", { name: /Token limits/ }));
    expect(
      screen.getByRole("slider", { name: "Auto-compression cap" }),
    ).toHaveAttribute("aria-valuemax", "100000");
    store.dispatch(
      applyChatEvent({
        chat_id: threadId,
        seq: "1",
        type: "thread_updated",
        context_tokens_cap: null,
      }),
    );
    await waitFor(() =>
      expect(
        screen.getByRole("slider", { name: "Auto-compression cap" }),
      ).toHaveAttribute("aria-valuemax", "200000"),
    );
    expect(
      screen.queryByRole("button", { name: "Reset request context limit" }),
    ).not.toBeInTheDocument();
    expect(
      store.getState().chat.threads[threadId]?.thread.auto_compression_cap,
    ).toBe(7777);
  });
});
