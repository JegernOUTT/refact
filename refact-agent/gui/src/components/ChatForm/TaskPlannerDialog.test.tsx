import { describe, expect, test, vi } from "vitest";
import { http, HttpResponse } from "msw";
import { render, screen, waitFor, within } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { createDefaultChatState } from "../../utils/test-utils";
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
  test("shows one content spinner and no button spinner while creating a task", async () => {
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
    expect(screen.queryByRole("progressbar")).not.toBeInTheDocument();
    expect(
      within(
        screen.getByRole("button", { name: "Creating task..." }),
      ).queryByRole("status"),
    ).not.toBeInTheDocument();

    onOpenChange.mockClear();
    await user.keyboard("{Escape}");
    expect(onOpenChange).not.toHaveBeenCalledWith(false);

    taskGate.resolve();
    await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false));
  });
});
