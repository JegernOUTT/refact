import { http, HttpResponse } from "msw";
import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "../utils/test-utils";
import { server } from "../utils/mockServer";
import { TaskList } from "../features/Tasks/TaskList";
import type { TaskMeta } from "../services/refact/tasks";

const CONFIG_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "web" as const,
  },
};

const TASK: TaskMeta = {
  id: "task-1",
  name: "New task",
  status: "planning",
  created_at: "2024-01-01T00:00:00Z",
  updated_at: "2024-01-01T00:00:00Z",
  cards_total: 0,
  cards_done: 0,
  cards_failed: 0,
  agents_active: 0,
};

describe("TaskCreate", () => {
  it("task_create_form_posts_target_files_array", async () => {
    let postedBody: unknown;
    server.use(
      http.get("*/v1/tasks", () => HttpResponse.json([])),
      http.post("*/v1/tasks", async ({ request }) => {
        postedBody = await request.json();
        return HttpResponse.json({
          id: "task-1",
          name: "New task",
          status: "planning",
          created_at: "2024-01-01T00:00:00Z",
          updated_at: "2024-01-01T00:00:00Z",
          cards_total: 0,
          cards_done: 0,
          cards_failed: 0,
          agents_active: 0,
        });
      }),
    );

    const { user } = render(<TaskList />, { preloadedState: CONFIG_STATE });

    await user.click(await screen.findByRole("button", { name: /new task/i }));
    await user.type(screen.getByPlaceholderText("Task name..."), "New task");
    await user.type(
      screen.getByLabelText("Target files"),
      "src/foo.rs, src/bar.ts",
    );
    await user.click(screen.getByRole("button", { name: /^create$/i }));

    await waitFor(() => {
      expect(postedBody).toEqual({
        name: "New task",
        target_files: ["src/foo.rs", "src/bar.ts"],
      });
    });
  });

  it("list_error_is_not_an_empty_list_and_retry_recovers", async () => {
    let fail = true;
    server.use(
      http.get("*/v1/tasks", () =>
        fail
          ? HttpResponse.json({ error: "List unavailable" }, { status: 500 })
          : HttpResponse.json([TASK]),
      ),
    );
    const { user } = render(<TaskList />, { preloadedState: CONFIG_STATE });
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "List unavailable",
    );
    expect(screen.queryByText(/No tasks yet/)).not.toBeInTheDocument();
    fail = false;
    await user.click(screen.getByRole("button", { name: "Retry tasks" }));
    expect(await screen.findByText(TASK.name)).toBeInTheDocument();
  });

  it("enter_and_click_cannot_duplicate_creation_and_failed_draft_survives_dismissal", async () => {
    let finishCreate: () => void = () => undefined;
    const pendingCreate = new Promise<void>((resolve) => {
      finishCreate = resolve;
    });
    const createCalls = vi.fn();
    server.use(
      http.get("*/v1/tasks", () => HttpResponse.json([])),
      http.post("*/v1/tasks", async () => {
        createCalls();
        await pendingCreate;
        return HttpResponse.json(
          { error: "Create unavailable" },
          { status: 500 },
        );
      }),
    );
    const { user } = render(<TaskList />, { preloadedState: CONFIG_STATE });
    await user.click(await screen.findByRole("button", { name: "New Task" }));
    await user.type(screen.getByPlaceholderText("Task name..."), "Draft task");
    await user.type(screen.getByLabelText("Target files"), "src/draft.ts");
    await user.type(screen.getByPlaceholderText("Task name..."), "{Enter}");
    await waitFor(() => expect(createCalls).toHaveBeenCalledOnce());
    const createButton = screen.getByRole("button", { name: "Creating..." });
    expect(createButton).toBeDisabled();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();
    await user.click(createButton);
    expect(createCalls).toHaveBeenCalledOnce();
    finishCreate();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Create unavailable",
    );
    expect(screen.getByPlaceholderText("Task name...")).toHaveValue(
      "Draft task",
    );
    expect(screen.getByLabelText("Target files")).toHaveValue("src/draft.ts");
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    await user.click(screen.getByRole("button", { name: "New Task" }));
    expect(screen.getByPlaceholderText("Task name...")).toHaveValue(
      "Draft task",
    );
    expect(screen.getByLabelText("Target files")).toHaveValue("src/draft.ts");
  });

  it("task_delete_cancel_and_pending_conflict_keep_the_task_without_force", async () => {
    let finishDelete: () => void = () => undefined;
    const pendingDelete = new Promise<void>((resolve) => {
      finishDelete = resolve;
    });
    const deleteCalls: string[] = [];
    server.use(
      http.get("*/v1/tasks", () => HttpResponse.json([TASK])),
      http.delete("*/v1/tasks/task-1", async ({ request }) => {
        deleteCalls.push(request.url);
        await pendingDelete;
        return HttpResponse.json(
          { error: "Task has active cards: T-1" },
          { status: 409 },
        );
      }),
    );
    const { user } = render(<TaskList />, { preloadedState: CONFIG_STATE });
    await user.click(
      await screen.findByRole("button", { name: "delete task" }),
    );
    let dialog = await screen.findByRole("dialog", { name: "Delete task" });
    expect(deleteCalls).toEqual([]);
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
    );
    expect(deleteCalls).toEqual([]);
    await user.click(screen.getByRole("button", { name: "delete task" }));
    dialog = await screen.findByRole("dialog", { name: "Delete task" });
    await user.click(
      within(dialog).getByRole("button", { name: "Delete task" }),
    );
    await waitFor(() => expect(deleteCalls).toHaveLength(1));
    const confirm = within(dialog).getByRole("button", { name: "Deleting..." });
    expect(confirm).toBeDisabled();
    await user.click(confirm);
    expect(deleteCalls).toHaveLength(1);
    expect(new URL(deleteCalls[0]).searchParams.has("force")).toBe(false);
    finishDelete();
    expect(await within(dialog).findByRole("alert")).toHaveTextContent("T-1");
    await user.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(await screen.findByText(TASK.name)).toBeInTheDocument();
  });
});
