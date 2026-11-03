import { describe, expect, test, vi } from "vitest";
import { http, HttpResponse } from "msw";
import { render, screen, waitFor } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { createDefaultChatState } from "../../utils/test-utils";
import { applyChatEvent } from "../../features/Chat/Thread/actions";
import { TaskPlannerDialog } from "./TaskPlannerDialog";

function deferred<T = void>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

describe("TaskPlannerDialog", () => {
  test("allows creating while waiting and disables creation while streaming", async () => {
    const chat = createDefaultChatState();
    const runtime = chat.threads[chat.current_thread_id];
    runtime.waiting_for_response = true;
    const { store } = render(
      <TaskPlannerDialog
        sourceChatId={chat.current_thread_id}
        open
        onOpenChange={vi.fn()}
      />,
      {
        preloadedState: {
          chat,
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

    expect(screen.getByRole("button", { name: "Create Task" })).toBeEnabled();

    store.dispatch(
      applyChatEvent({
        chat_id: chat.current_thread_id,
        seq: "1",
        type: "runtime_updated",
        state: "generating",
      }),
    );

    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Create Task" }),
      ).toBeDisabled(),
    );
  });

  test("blocks closing while creating a task and closes when done", async () => {
    const taskGate = deferred();
    const onOpenChange = vi.fn();

    server.use(
      http.post("*/v1/tasks", async () => {
        await taskGate.promise;
        return HttpResponse.json({
          id: "task-1",
          name: "New Task",
          status: "planning",
          created_at: "2026-01-01T00:00:00Z",
          updated_at: "2026-01-01T00:00:00Z",
          cards_total: 0,
          cards_done: 0,
          cards_failed: 0,
          agents_active: 0,
        });
      }),
      http.post("*/v1/tasks/task-1/planner-chats", () =>
        HttpResponse.json({ chat_id: "planner-chat" }),
      ),
    );

    const chat = createDefaultChatState();

    const { user } = render(
      <TaskPlannerDialog
        sourceChatId={chat.current_thread_id}
        open
        onOpenChange={onOpenChange}
      />,
      {
        preloadedState: {
          chat,
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

    await user.click(screen.getByRole("button", { name: "Create Task" }));

    expect(await screen.findByRole("status")).toHaveTextContent(
      "Creating task...",
    );

    onOpenChange.mockClear();
    await user.keyboard("{Escape}");
    expect(onOpenChange).not.toHaveBeenCalledWith(false);

    taskGate.resolve();
    await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false));
  });
});
