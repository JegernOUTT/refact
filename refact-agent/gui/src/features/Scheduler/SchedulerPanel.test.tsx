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
});
