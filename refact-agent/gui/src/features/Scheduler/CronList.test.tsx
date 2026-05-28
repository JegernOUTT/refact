import { describe, expect, it, vi } from "vitest";
import { screen, render } from "../../utils/test-utils";
import { CronList } from "./CronList";
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

describe("CronList", () => {
  it("renders cron task fixture", () => {
    render(<CronList tasks={[task]} onDelete={vi.fn()} />);

    expect(screen.getByText("hourly at :07")).toBeInTheDocument();
    expect(screen.getByText("7 * * * *")).toBeInTheDocument();
    expect(screen.getByText("Hourly frog check")).toBeInTheDocument();
    expect(screen.getByText("Durable")).toBeInTheDocument();
    expect(screen.getByText("Recurring")).toBeInTheDocument();
    expect(screen.getByText("3")).toBeInTheDocument();
  });

  it("calls delete with the task id", async () => {
    const onDelete = vi.fn();
    const { user } = render(<CronList tasks={[task]} onDelete={onDelete} />);

    await user.click(screen.getByRole("button", { name: "Delete" }));

    expect(onDelete).toHaveBeenCalledWith("cron_1");
  });
});
