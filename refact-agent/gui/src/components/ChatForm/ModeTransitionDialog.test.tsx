import { describe, expect, test, vi } from "vitest";
import { http, HttpResponse } from "msw";
import { render, screen, waitFor } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { createDefaultChatState } from "../../utils/test-utils";
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

describe("ModeTransitionDialog", () => {
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

  test("closes after opening the new chat when starting the assistant fails", async () => {
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

    await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false));

    const newThread = store.getState().chat.threads["new-chat"];
    expect(store.getState().chat.current_thread_id).toBe("new-chat");
    expect(newThread?.error).toContain(
      "Failed to start assistant after mode switch",
    );
  });
});
