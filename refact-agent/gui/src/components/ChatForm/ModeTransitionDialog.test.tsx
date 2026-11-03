import { describe, expect, test, vi } from "vitest";
import { http, HttpResponse } from "msw";
import { render, screen, waitFor } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { createDefaultChatState } from "../../utils/test-utils";
import {
  applyChatEvent,
  setIsWaitingForResponse,
} from "../../features/Chat/Thread/actions";
import { ModeTransitionDialog } from "./ModeTransitionDialog";

function deferred<T = void>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function chatStateWithSourceThread() {
  const chat = createDefaultChatState();
  const originalId = chat.current_thread_id;
  const runtime = chat.threads[originalId];
  runtime.thread.id = "source-chat";
  runtime.thread.mode = "agent";
  runtime.thread.messages = [
    { role: "user", content: "hello", message_id: "user-1" },
  ];
  chat.current_thread_id = "source-chat";
  chat.open_thread_ids = ["source-chat"];
  chat.threads = { "source-chat": runtime };
  return chat;
}

const testConfig = {
  apiKey: "test",
  host: "web" as const,
  dev: true,
  lspPort: 8001,
  themeProps: {},
};

function renderDialog(chat: ReturnType<typeof chatStateWithSourceThread>) {
  return render(
    <ModeTransitionDialog
      open
      onOpenChange={vi.fn()}
      chatId="source-chat"
      currentMode="agent"
      targetMode="ask"
      targetModeTitle="Ask"
      targetModeDescription="Quick answers"
    />,
    { preloadedState: { chat, config: testConfig } },
  );
}

function installSuccessfulTransitionHandlers(onApply?: () => void) {
  server.use(
    http.post("*/v1/chats/source-chat/trajectory/mode-transition/apply", () => {
      onApply?.();
      return HttpResponse.json({
        new_chat_id: "new-chat",
        messages_count: 1,
      });
    }),
    http.get("*/v1/trajectories/all", () => HttpResponse.json([])),
    http.post("*/v1/chats/new-chat/commands", () =>
      HttpResponse.json({ status: "queued" }),
    ),
  );
}

describe("ModeTransitionDialog", () => {
  test("switches while the source awaits a question answer and keeps the source thread", async () => {
    let applied = false;
    installSuccessfulTransitionHandlers(() => {
      applied = true;
    });
    const chat = chatStateWithSourceThread();
    const runtime = chat.threads["source-chat"];
    runtime.snapshot_received = true;
    runtime.thread.pending_questions = [
      {
        tool_call_id: "ask-1",
        questions: [{ id: "q1", type: "yes_no", text: "Continue?" }],
        asked_at_ms: 1,
        status: "pending",
      },
    ];

    const { user, store } = renderDialog(chat);
    store.dispatch(
      applyChatEvent({
        chat_id: "source-chat",
        seq: "1",
        type: "runtime_updated",
        state: "waiting_user_input",
      }),
    );
    const button = screen.getByRole("button", { name: "Switch Mode" });
    expect(button).toBeEnabled();
    await user.click(button);

    await waitFor(() =>
      expect(store.getState().chat.current_thread_id).toBe("new-chat"),
    );
    expect(applied).toBe(true);
    expect(store.getState().chat.threads["source-chat"]).toBeDefined();
    expect(store.getState().chat.open_thread_ids).toEqual(
      expect.arrayContaining(["source-chat", "new-chat"]),
    );
  });

  test("switches from an idle source and closes the source thread", async () => {
    installSuccessfulTransitionHandlers();
    const chat = chatStateWithSourceThread();
    chat.threads["source-chat"].snapshot_received = true;
    const { user, store } = renderDialog(chat);
    store.dispatch(
      applyChatEvent({
        chat_id: "source-chat",
        seq: "1",
        type: "runtime_updated",
        state: "idle",
      }),
    );

    await user.click(screen.getByRole("button", { name: "Switch Mode" }));

    await waitFor(() =>
      expect(store.getState().chat.current_thread_id).toBe("new-chat"),
    );
    expect(store.getState().chat.threads["source-chat"]).toBeUndefined();
    expect(store.getState().chat.open_thread_ids).not.toContain("source-chat");
    expect(store.getState().chat.open_thread_ids).toContain("new-chat");
  });

  test("disables switching once the source starts streaming", async () => {
    let applied = false;
    server.use(
      http.post(
        "*/v1/chats/source-chat/trajectory/mode-transition/apply",
        () => {
          applied = true;
          return HttpResponse.json({
            new_chat_id: "new-chat",
            messages_count: 1,
          });
        },
      ),
    );
    const chat = chatStateWithSourceThread();
    chat.threads["source-chat"].snapshot_received = true;
    const { user, store } = renderDialog(chat);
    store.dispatch(setIsWaitingForResponse({ id: "source-chat", value: true }));
    expect(screen.getByRole("button", { name: "Switch Mode" })).toBeEnabled();

    store.dispatch(
      applyChatEvent({
        chat_id: "source-chat",
        seq: "1",
        type: "runtime_updated",
        state: "generating",
      }),
    );
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Switch Mode" }),
      ).toBeDisabled(),
    );
    await user.click(screen.getByRole("button", { name: "Switch Mode" }));
    expect(applied).toBe(false);
    expect(store.getState().chat.current_thread_id).toBe("source-chat");
  });

  test("keeps the dialog open while the new chat is starting", async () => {
    const commandGate = deferred();
    const onOpenChange = vi.fn();
    let regenerateRequested = false;

    server.use(
      http.post("*/v1/chats/source-chat/trajectory/mode-transition/apply", () =>
        HttpResponse.json({ new_chat_id: "new-chat", messages_count: 1 }),
      ),
      http.get("*/v1/trajectories/all", () => HttpResponse.json([])),
      http.post("*/v1/chats/new-chat/commands", async () => {
        regenerateRequested = true;
        await commandGate.promise;
        return HttpResponse.json({ status: "queued" });
      }),
    );

    const { user, store } = render(
      <ModeTransitionDialog
        open
        onOpenChange={onOpenChange}
        chatId="source-chat"
        currentMode="agent"
        targetMode="ask"
        targetModeTitle="Ask"
        targetModeDescription="Quick answers"
      />,
      {
        preloadedState: {
          chat: chatStateWithSourceThread(),
          config: {
            apiKey: "test",
            host: "web",
            dev: true,
            lspPort: 8001,
            themeProps: {},
          },
        },
      },
    );

    await user.click(screen.getByRole("button", { name: "Switch Mode" }));

    await waitFor(() => expect(regenerateRequested).toBe(true));
    expect(onOpenChange).not.toHaveBeenCalledWith(false);
    expect(screen.queryByRole("progressbar")).not.toBeInTheDocument();
    expect(screen.getByRole("status")).toHaveTextContent(
      "Starting assistant...",
    );
    expect(store.getState().chat.current_thread_id).toBe("new-chat");

    commandGate.resolve();

    await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false));
  });

  test("closes on the new chat when starting the assistant fails", async () => {
    const onOpenChange = vi.fn();
    server.use(
      http.post("*/v1/chats/source-chat/trajectory/mode-transition/apply", () =>
        HttpResponse.json({ new_chat_id: "new-chat", messages_count: 1 }),
      ),
      http.get("*/v1/trajectories/all", () => HttpResponse.json([])),
      http.post("*/v1/chats/new-chat/commands", () =>
        HttpResponse.json({ detail: "queue failed" }, { status: 500 }),
      ),
    );
    const { user, store } = render(
      <ModeTransitionDialog
        open
        onOpenChange={onOpenChange}
        chatId="source-chat"
        currentMode="agent"
        targetMode="ask"
        targetModeTitle="Ask"
        targetModeDescription="Quick answers"
      />,
      {
        preloadedState: {
          chat: chatStateWithSourceThread(),
          config: testConfig,
        },
      },
    );

    await user.click(screen.getByRole("button", { name: "Switch Mode" }));

    await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false));
    expect(store.getState().chat.current_thread_id).toBe("new-chat");
    expect(store.getState().chat.threads["new-chat"]?.error).toContain(
      "Failed to start assistant after mode switch",
    );
  });
});
