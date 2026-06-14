import { http, HttpResponse, delay } from "msw";
import { describe, expect, it, vi } from "vitest";
import { screen, render, waitFor } from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { SchedulerPanel } from "./SchedulerPanel";
import type { CronTask } from "../../services/refact/schedulerApi";

const task: CronTask = {
  id: "cron_1",
  cron: "7 * * * *",
  human_schedule: "hourly at :07",
  description: "Hourly frog check",
  prompt: "Check frogs",
  recurring: true,
  durable: true,
  next_fire_at_ms: Date.UTC(2026, 0, 1, 9, 7),
  fire_count: 3,
  created_at_ms: Date.UTC(2026, 0, 1, 8, 0),
  enabled: true,
  paused: false,
  trigger_kind: "cron",
  tz: "UTC",
  every_ms: null,
  at_ms: null,
  last_status: "fired",
  last_error: null,
  recent_runs: [
    {
      at_ms: Date.UTC(2026, 0, 1, 8, 7),
      status: "fired",
      error: null,
    },
  ],
  action_kind: "agent_turn",
  chat_id: "chat-1",
  target: "existing_chat",
  isolated: false,
};

describe("SchedulerPanel", () => {
  it("catches failed delete requests and clears deleting state", async () => {
    const unhandledRejection = vi.fn();
    window.addEventListener("unhandledrejection", unhandledRejection);
    server.use(
      http.get("*/v1/scheduler/cron", () => HttpResponse.json([task])),
      http.delete("*/v1/scheduler/cron/:id", async () => {
        await delay(25);
        return HttpResponse.json(
          { detail: "Cannot delete stale cron task" },
          { status: 500 },
        );
      }),
    );

    try {
      const { user } = render(<SchedulerPanel onBack={vi.fn()} embedded />);

      expect(await screen.findByText("hourly at :07")).toBeInTheDocument();
      const deleteButton = screen.getByRole("button", { name: "Delete" });
      await user.click(deleteButton);

      await waitFor(() => expect(deleteButton).toBeDisabled());
      expect(await screen.findByRole("alert")).toHaveTextContent(
        "Cannot delete stale cron task",
      );
      await waitFor(() => expect(deleteButton).not.toBeDisabled());
      expect(unhandledRejection).not.toHaveBeenCalled();
    } finally {
      window.removeEventListener("unhandledrejection", unhandledRejection);
    }
  });

  it("wires pause, run-now, and edit mutations", async () => {
    const patchBodies: unknown[] = [];
    const runIds: string[] = [];
    server.use(
      http.get("*/v1/scheduler/cron", () => HttpResponse.json([task])),
      http.patch("*/v1/scheduler/cron/:id", async ({ request }) => {
        patchBodies.push(await request.json());
        return HttpResponse.json({
          id: "cron_1",
          updated: true,
          human_schedule: "hourly at :07",
        });
      }),
      http.post("*/v1/scheduler/cron/:id/run", ({ params }) => {
        runIds.push(String(params.id));
        return HttpResponse.json({ id: "cron_1", triggered: true });
      }),
    );

    const { user } = render(<SchedulerPanel onBack={vi.fn()} embedded />);

    expect(await screen.findByText("hourly at :07")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Pause" }));
    await waitFor(() => expect(patchBodies).toContainEqual({ enabled: false }));

    await user.click(screen.getByRole("button", { name: "Run now" }));
    await waitFor(() => expect(runIds).toContain("cron_1"));

    await user.click(screen.getByRole("button", { name: "Edit" }));
    await user.clear(screen.getByLabelText("Edit description"));
    await user.type(screen.getByLabelText("Edit description"), "Updated frogs");
    await user.clear(screen.getByLabelText("Edit cron expression"));
    await user.type(
      screen.getByLabelText("Edit cron expression"),
      "*/15 * * * *",
    );
    await user.clear(screen.getByLabelText("Edit timezone"));
    await user.type(screen.getByLabelText("Edit timezone"), "Asia/Tokyo");
    await user.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => {
      expect(patchBodies).toContainEqual({
        description: "Updated frogs",
        cron: "*/15 * * * *",
        tz: "Asia/Tokyo",
      });
    });
  });
});
