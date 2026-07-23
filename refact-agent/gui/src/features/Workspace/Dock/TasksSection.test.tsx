import { http, HttpResponse } from "msw";
import { describe, expect, it, vi } from "vitest";

import type {
  BoardCard,
  TaskBoard,
  TaskMeta,
} from "../../../services/refact/tasks";
import { render, screen, waitFor, within } from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { TasksSection, TasksSectionView } from "./TasksSection";
import { buildTaskDockEntries, type TaskDockEntry } from "./tasksSectionModel";

const task = (overrides: Partial<TaskMeta> = {}): TaskMeta => ({
  id: "task-a",
  name: "Workspace relayout",
  status: "active",
  created_at: "2026-07-23T00:00:00Z",
  updated_at: "2026-07-23T01:00:00Z",
  cards_total: 1,
  cards_done: 0,
  cards_failed: 0,
  agents_active: 1,
  ...overrides,
});

const card = (overrides: Partial<BoardCard> = {}): BoardCard => ({
  id: "W-4",
  title: "Add Tasks dock section",
  column: "doing",
  priority: "P1",
  depends_on: [],
  instructions: "Reuse the existing board.",
  assignee: "agent-1",
  agent_chat_id: "agent-W-4",
  status_updates: [],
  final_report: null,
  created_at: "2026-07-23T00:00:00Z",
  started_at: "2026-07-23T00:10:00Z",
  completed_at: null,
  last_heartbeat_at: "2026-07-23T00:20:00Z",
  target_files: [],
  ...overrides,
});

const board = (cards: BoardCard[]): TaskBoard => ({
  schema_version: 1,
  rev: 1,
  columns: [
    { id: "doing", title: "In progress" },
    { id: "done", title: "Done" },
    { id: "failed", title: "Failed" },
  ],
  cards,
});

const entry = (
  title: string,
  agentStatus: TaskDockEntry["agentStatus"],
  recencyAt: number,
): TaskDockEntry => ({
  cardId: title,
  taskId: "task-a",
  taskName: "Workspace relayout",
  title,
  columnLabel: agentStatus,
  agentStatus,
  recencyAt,
});

describe("TasksSection", () => {
  it("orders attention first then recency and shows header attention", () => {
    const onOpenTask = vi.fn();
    render(
      <TasksSectionView
        entries={[
          entry("Running newer", "running", 300),
          entry("Failed older", "failed", 100),
          entry("Done newest", "done", 400),
          entry("Stuck newer", "stuck", 200),
        ]}
        isLoading={false}
        onOpenBoard={vi.fn()}
        onOpenTask={onOpenTask}
      />,
    );

    expect(screen.getByLabelText("Tasks need attention")).toBeInTheDocument();
    expect(
      screen
        .getAllByRole("button")
        .map(
          (button) =>
            within(button).getByText(/newer|older|newest/i).textContent,
        ),
    ).toEqual(["Stuck newer", "Failed older", "Done newest", "Running newer"]);

    screen.getByRole("button", { name: /Stuck newer/ }).click();
    expect(onOpenTask).toHaveBeenCalledWith(
      expect.objectContaining({ title: "Stuck newer", agentStatus: "stuck" }),
    );
  });

  it("derives live card statuses and excludes unassigned cards", () => {
    const entries = buildTaskDockEntries(
      [task()],
      {
        "task-a": board([
          card({ id: "stuck", title: "Stuck card" }),
          card({
            id: "failed",
            title: "Failed card",
            column: "failed",
          }),
          card({
            id: "unassigned",
            title: "Unassigned card",
            assignee: null,
            agent_chat_id: null,
          }),
        ]),
      },
      Date.parse("2026-07-23T01:00:01Z"),
    );

    expect(entries).toEqual([
      expect.objectContaining({ title: "Stuck card", agentStatus: "stuck" }),
      expect.objectContaining({ title: "Failed card", agentStatus: "failed" }),
    ]);
  });

  it("shows the empty state and opens the existing Tasks board list", async () => {
    server.use(http.get("*/v1/tasks", () => HttpResponse.json([])));
    const view = render(<TasksSection />);

    expect(await screen.findByText("No active tasks")).toBeInTheDocument();
    await view.user.click(screen.getByRole("button", { name: "Open board" }));

    expect(view.store.getState().pages.at(-1)).toEqual({ name: "tasks list" });
  });

  it("opens the selected card's existing TaskWorkspace board surface", async () => {
    server.use(
      http.get("*/v1/tasks", () => HttpResponse.json([task()])),
      http.get("*/v1/tasks/task-a/board", () =>
        HttpResponse.json(
          board([
            card({
              last_heartbeat_at: new Date().toISOString(),
            }),
          ]),
        ),
      ),
    );
    const view = render(<TasksSection />);

    await view.user.click(
      await screen.findByRole("button", { name: /Add Tasks dock section/ }),
    );

    await waitFor(() => {
      expect(view.store.getState().pages.at(-1)).toEqual({
        name: "task workspace",
        taskId: "task-a",
      });
    });
    expect(view.store.getState().tasksUI.openTasks).toEqual([
      expect.objectContaining({ id: "task-a", name: "Workspace relayout" }),
    ]);
  });
});
